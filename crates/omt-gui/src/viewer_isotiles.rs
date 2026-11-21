//! **Streamed isometric tiles.** The isometric view is cut into fixed,
//! world-anchored screen tiles ([`omt_render::iso::TileSpec`]): panning
//! only moves where the finished tiles are drawn (a pure translation, glued
//! to the cursor), tiles one ring beyond the window edge are rendered ahead
//! of time in the background, and tiles that scroll two rings away are
//! dropped (their GPU uploads freed). Zooming keeps the previous zoom's
//! tiles on screen, scaled, until the new ones cover the view.
//!
//! Each tile reads the loaded map tiles through a shared [`Snapshot`]
//! (reference counts only); a tile that sampled a map tile that was still
//! loading is re-rendered when that map tile arrives — the old image stays
//! up until the new one is ready, so nothing flashes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use iced::Task;
use iced::widget::image::{Allocation, Handle};
use omt_render::TileKey;
use omt_render::blocks3d::Kind;
use omt_render::iso::{TileSpec, paint_tile, world_box};

use crate::session::{Store, blocking};
use crate::viewer::{ViewerMsg, ViewerState};
use crate::viewer_3d::Snapshot;

/// Tile edge in logical pixels.
pub const TILE: f32 = 256.0;

/// Tile index in the global iso plane.
pub type TKey = (i32, i32);

/// A finished tile (its GPU upload lives as long as it does).
pub struct IsoTile {
    pub handle: Handle,
    _alloc: Option<Allocation>,
    /// Map tiles it wanted that were not loaded yet.
    missing: Vec<TileKey>,
}

/// What a set of tiles was rendered for: tiles of different layers never
/// mix (a different zoom is only a scaled fallback).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub rot: u8,
    pub scale: f32,
    pub vscale: f32,
    pub textured: bool,
    /// Rendered pixels per tile edge (render quality).
    pub px: u32,
    pub sky: [u8; 3],
    /// Texture pack generation (textured detail).
    pub pack: u64,
}

/// A finished tile job.
#[derive(Debug, Clone)]
pub struct TileDone {
    pub epoch: u64,
    pub key: TKey,
    pub handle: Handle,
    pub missing: Vec<TileKey>,
}

#[derive(Default)]
pub struct IsoTiles {
    pub layer: Option<Layer>,
    pub tiles: HashMap<TKey, IsoTile>,
    /// The previous zoom's tiles and their scale, drawn scaled under the
    /// current ones until those cover the view.
    pub prev: Option<(f32, HashMap<TKey, IsoTile>)>,
    inflight: HashSet<TKey>,
    /// Tiles whose map data has arrived since they were drawn.
    stale: HashSet<TKey>,
    epoch: u64,
    snap: Option<Arc<Snapshot>>,
    /// Block looks for the textured painter, shared by every tile job.
    kinds: Arc<Mutex<HashMap<String, Kind>>>,
    /// Map-loader area last requested (world box, level), to skip
    /// redundant refills while panning inside it.
    area: Option<((i64, i64, i64, i64), u8)>,
    /// Debug/testing hook (`OMT_DEBUG_ISO_PAN="dx,dy;*1.25;dx,dy"`): each
    /// pan (or `*factor` zoom) is applied once the view has no tile in
    /// flight (headless drag / zoom tests); timings go to stderr.
    debug_pan: Option<std::collections::VecDeque<(f32, f32)>>,
    since: Option<std::time::Instant>,
    /// Debug/testing hook (`OMT_DEBUG_ISO_DRAG="dx,dy,frames"`): once the
    /// first view settled, a 60 Hz stream of drag events (the canvas's own
    /// `Ev3::Move`) — measures per-event UI cost and unrendered area.
    pub drag: Option<DebugDrag>,
    /// Smoothed camera motion per drag event (global px): prefetch reaches
    /// further ahead in the direction of a fast drag.
    pub vel: (f32, f32),
    /// Tiles drawn in flat colors during a fast drag (textured detail):
    /// they re-render textured once the view keeps up.
    quick: HashSet<TKey>,
}

/// State + stats of the synthetic drag.
#[derive(Debug, Default)]
pub struct DebugDrag {
    pub step: (f32, f32),
    pub left: u32,
    pub started: bool,
    frames: u32,
    total: std::time::Duration,
    max: std::time::Duration,
    /// Frames with any visible tile not rendered yet, and the worst share.
    gap_frames: u32,
    worst_gap: f32,
}

impl IsoTiles {
    /// The synthetic drag step to emit now (`None` = idle).
    pub fn drag_step(&self) -> Option<(f32, f32)> {
        self.drag.as_ref().filter(|d| d.started && d.left > 0).map(|d| d.step)
    }

    /// Record one handled drag event: its UI-thread cost and the share of
    /// visible tiles not rendered (would show background).
    pub fn drag_record(&mut self, cost: std::time::Duration, missing: f32) {
        let Some(d) = self.drag.as_mut().filter(|d| d.started && d.left > 0) else { return };
        d.left -= 1;
        d.frames += 1;
        d.total += cost;
        d.max = d.max.max(cost);
        if missing > 0.0 {
            d.gap_frames += 1;
            d.worst_gap = d.worst_gap.max(missing);
        }
        if d.left == 0 {
            eprintln!(
                "iso drag: {} events, UI cost avg {:.0} µs / max {:.0} µs, frames with unrendered tiles {} (worst {:.1}% of the view)",
                d.frames,
                d.total.as_secs_f64() * 1e6 / d.frames.max(1) as f64,
                d.max.as_secs_f64() * 1e6,
                d.gap_frames,
                d.worst_gap * 100.0
            );
        }
    }
}

impl IsoTiles {
    /// Forget everything (leaving the iso view).
    pub fn clear(&mut self) {
        *self = Self { kinds: Arc::clone(&self.kinds), epoch: self.epoch + 1, ..Default::default() };
    }

    /// The map data changed (style, texture pack): re-render every tile,
    /// keeping the current images up as a fallback meanwhile.
    pub fn invalidate(&mut self) {
        self.snap = None;
        self.stale.extend(self.tiles.keys().copied());
        self.kinds.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// A map tile finished loading: tiles that missed it re-render.
    pub fn map_tile(&mut self, key: TileKey) {
        self.snap = None;
        for (k, t) in &self.tiles {
            if t.missing.contains(&key) {
                self.stale.insert(*k);
            }
        }
    }

    /// Switch to `layer`. A zoom keeps the old tiles as a scaled fallback;
    /// anything else (view angle, detail, quality) starts over.
    fn set_layer(&mut self, layer: Layer) {
        let Some(old) = self.layer else {
            self.layer = Some(layer);
            return;
        };
        if old == layer {
            return;
        }
        self.epoch += 1;
        self.inflight.clear();
        if (Layer { pack: layer.pack, textured: layer.textured, sky: layer.sky, ..old }) == layer {
            // Same geometry, new look (textures, detail, background): keep
            // the tiles up and redo them in place.
            self.layer = Some(layer);
            self.kinds.lock().unwrap_or_else(|e| e.into_inner()).clear();
            self.stale.extend(self.tiles.keys().copied());
            return;
        }
        self.stale.clear();
        self.quick.clear();
        let zoom_only = Layer { scale: layer.scale, ..old } == layer;
        let tiles = std::mem::take(&mut self.tiles);
        if zoom_only {
            // Keep whichever fallback has more to show.
            let keep_old = self.prev.as_ref().is_some_and(|(_, p)| p.len() > tiles.len());
            if !keep_old {
                self.prev = (!tiles.is_empty()).then_some((old.scale, tiles));
            }
        } else {
            self.prev = None;
            self.kinds.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
        self.layer = Some(layer);
    }

    /// Tiles to draw: `(x, y, edge)` in logical canvas pixels, the previous
    /// zoom's (scaled) first.
    pub fn draw_list(&self, center: (f64, f64), view: (f32, f32)) -> Vec<(f32, f32, f32, Handle)> {
        let Some(layer) = self.layer else { return Vec::new() };
        let ox = (view.0 as f64 / 2.0 - center.0).round();
        let oy = (view.1 as f64 / 2.0 - center.1).round();
        let mut out = Vec::new();
        let mut put = |tiles: &HashMap<TKey, IsoTile>, k: f64| {
            let edge = TILE as f64 * k;
            for (&(i, j), t) in tiles {
                let (x, y) = (ox + i as f64 * edge, oy + j as f64 * edge);
                if x + edge < 0.0 || y + edge < 0.0 || x > view.0 as f64 || y > view.1 as f64 {
                    continue;
                }
                out.push((x as f32, y as f32, edge as f32, t.handle.clone()));
            }
        };
        if let Some((s0, prev)) = &self.prev {
            put(prev, (layer.scale / s0) as f64);
        }
        put(&self.tiles, 1.0);
        out
    }

    /// A tile set for tests: blank tiles at `keys` in `layer`.
    #[cfg(test)]
    pub fn for_test(layer: Layer, keys: &[TKey]) -> Self {
        let tiles = keys.iter().map(|&k| (k, IsoTile { handle: Handle::from_rgba(1, 1, vec![0; 4]), _alloc: None, missing: Vec::new() })).collect();
        Self { layer: Some(layer), tiles, ..Default::default() }
    }

    /// Tiles still being rendered.
    pub fn pending(&self) -> usize {
        self.inflight.len()
    }
}

/// Inclusive tile index range covering `[lo, hi)` logical pixels.
fn range(lo: f64, hi: f64) -> (i32, i32) {
    ((lo / TILE as f64).floor() as i32, ((hi / TILE as f64).ceil() as i32 - 1).max((lo / TILE as f64).floor() as i32))
}

impl ViewerState {
    /// The layer the iso view currently shows.
    fn iso_layer(&self) -> Layer {
        let q = self.prefs.render_scale.clamp(0.25, 1.0);
        Layer {
            rot: self.v3.iso_cam.rot,
            scale: self.v3.iso_cam.scale,
            vscale: self.v3.iso_cam.vscale,
            textured: self.v3.iso_textured,
            px: (TILE * q).round() as u32,
            // Void reads as void (the 2D map's background), never as water.
            sky: {
                let c = crate::style::t().map_bg;
                [(c.r * 255.0) as u8, (c.g * 255.0) as u8, (c.b * 255.0) as u8]
            },
            pack: if self.v3.iso_textured { crate::textures::generation() } else { 0 },
        }
    }

    /// Share of the visible view (tile count) not rendered yet.
    pub(crate) fn iso_missing(&self) -> f32 {
        let Some(layer) = self.isot.layer else { return 0.0 };
        let c = self.v3.iso_cam.center_px(layer.scale as f64);
        let (w, h) = (self.v3.size.0.max(1) as f64, self.v3.size.1.max(1) as f64);
        let (x0, x1) = range(c.0 - w / 2.0, c.0 + w / 2.0);
        let (y0, y1) = range(c.1 - h / 2.0, c.1 + h / 2.0);
        let total = ((x1 - x0 + 1) * (y1 - y0 + 1)).max(1) as f32;
        let missing = (y0..=y1).flat_map(|j| (x0..=x1).map(move |i| (i, j))).filter(|k| !self.isot.tiles.contains_key(k)).count();
        missing as f32 / total
    }

    /// Plan the iso tiles around the view: drop far ones, point the map
    /// loader at the data under them, and start background renders
    /// (visible tiles first, nearest the centre first).
    pub(crate) fn iso_pump(&mut self, store: Option<&Store>) -> Task<ViewerMsg> {
        if self.v3.iso.is_none() || !self.v3.on {
            return Task::none();
        }
        let layer = self.iso_layer();
        self.isot.set_layer(layer);
        let s = layer.scale as f64;
        let center = self.v3.iso_cam.center_px(s);
        let (w, h) = (self.v3.size.0.max(1) as f64, self.v3.size.1.max(1) as f64);
        let (x0, x1) = range(center.0 - w / 2.0, center.0 + w / 2.0);
        let (y0, y1) = range(center.1 - h / 2.0, center.1 + h / 2.0);
        // Prefetch one ring past the window edge, plus up to three more
        // tiles ahead of a fast drag (~¼ s of motion); despawn one ring
        // beyond that.
        let ahead = |v: f32| ((v.abs() * 15.0 / TILE).ceil() as i32).min(3);
        let (ax, ay) = (ahead(self.isot.vel.0), ahead(self.isot.vel.1));
        let (wx0, wx1) = (x0 - 1 - if self.isot.vel.0 < 0.0 { ax } else { 0 }, x1 + 1 + if self.isot.vel.0 > 0.0 { ax } else { 0 });
        let (wy0, wy1) = (y0 - 1 - if self.isot.vel.1 < 0.0 { ay } else { 0 }, y1 + 1 + if self.isot.vel.1 > 0.0 { ay } else { 0 });
        let keep = |&(i, j): &TKey| i >= wx0 - 1 && i <= wx1 + 1 && j >= wy0 - 1 && j <= wy1 + 1;
        self.isot.tiles.retain(|k, _| keep(k));
        self.isot.stale.retain(|k| keep(k));
        self.isot.quick.retain(|k| keep(k));
        let visible_done = (y0..=y1).all(|j| (x0..=x1).all(|i| self.isot.tiles.contains_key(&(i, j))));
        if visible_done {
            self.isot.prev = None;
        }

        // Map data under the wanted tiles (prefetch ring included).
        let snap_hl = self.isot.snap.as_ref().map(|sn| (sn.low, sn.high));
        let (low, high) = snap_hl.unwrap_or((0.0, 128.0));
        let t = TILE as f64;
        let wb = world_box(layer.rot, s, layer.vscale as f64, wx0 as f64 * t, wy0 as f64 * t, (wx1 + 1) as f64 * t, (wy1 + 1) as f64 * t, low as f64, high as f64, 8.0);
        let level = if s >= 1.0 { 0 } else { ((1.0 / s).log2().floor() as u8).min(omt_render::MAX_LEVEL) };
        // Quantize to 64 blocks so small pans don't refill the loader.
        let q = |v: f64| (v / 64.0).floor() as i64;
        let akey = ((q(wb.0), q(wb.1), q(wb.2) + 1, q(wb.3) + 1), level);
        let mut tasks = Vec::new();
        if self.isot.area != Some(akey) {
            self.isot.area = Some(akey);
            let ((a, b, c, d), l) = akey;
            self.ctl.area = Some((((a * 64) as f64, (b * 64) as f64, (c * 64) as f64, (d * 64) as f64), l));
            tasks.push(self.ctl.refill(store).map(ViewerMsg::Tiles));
        }

        let Some(map) = self.ctl.map.as_ref() else { return Task::batch(tasks) };
        let snap = match &self.isot.snap {
            Some(s) => Arc::clone(s),
            None => {
                let sn = Arc::new(Snapshot::of(map));
                self.isot.snap = Some(Arc::clone(&sn));
                sn
            }
        };
        if snap.is_empty() {
            return Task::batch(tasks);
        }
        // Absent tiles first (visible, then the ring), then stale ones;
        // nearest the centre first within each group.
        let (ci, cj) = (center.0 / t, center.1 / t);
        let mut todo: Vec<(u8, f64, TKey)> = Vec::new();
        for j in wy0..=wy1 {
            for i in wx0..=wx1 {
                let k = (i, j);
                if self.isot.inflight.contains(&k) {
                    continue;
                }
                let vis = i >= x0 && i <= x1 && j >= y0 && j <= y1;
                let group = match (self.isot.tiles.contains_key(&k), self.isot.stale.contains(&k)) {
                    (false, _) => u8::from(!vis),
                    (true, true) => 2 + u8::from(!vis),
                    (true, false) => continue,
                };
                let d = (i as f64 + 0.5 - ci).powi(2) + (j as f64 + 0.5 - cj).powi(2);
                todo.push((group, d, k));
            }
        }
        todo.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let max_jobs = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(2, 6));
        let slots = max_jobs.saturating_sub(self.isot.inflight.len());
        let src = map.source.clone();
        // Fast drag in textured detail: new tiles come in flat colors first
        // (several times cheaper), the textured pass follows as a refresh.
        let fast = self.isot.vel.0.hypot(self.isot.vel.1) > 20.0;
        for &(group, _, key) in todo.iter().take(slots) {
            self.isot.inflight.insert(key);
            self.isot.stale.remove(&key);
            let quick = layer.textured && fast && group < 2;
            if quick {
                self.isot.quick.insert(key);
            } else {
                self.isot.quick.remove(&key);
            }
            let spec = TileSpec {
                rot: layer.rot,
                scale: s * layer.px as f64 / TILE as f64,
                vscale: layer.vscale as f64,
                px: key.0 as f64 * layer.px as f64,
                py: key.1 as f64 * layer.px as f64,
                size: layer.px as usize,
                low: snap.low,
                high: snap.high,
            };
            let job = TileJob { spec, snap: Arc::clone(&snap), kinds: Arc::clone(&self.isot.kinds), textured: layer.textured && !quick, sky: layer.sky, level, src: src.clone() };
            let epoch = self.isot.epoch;
            tasks.push(Task::perform(blocking(move || job.run()), move |(img, missing)| (img, missing)).then(move |(img, missing): (Handle, Vec<TileKey>)| {
                let shown = img.clone();
                iced::widget::image::allocate(img).map(move |a| ViewerMsg::IsoTile(TileDone { epoch, key, handle: shown.clone(), missing: missing.clone() }, a.ok()))
            }));
        }
        Task::batch(tasks)
    }

    /// A tile render finished.
    pub(crate) fn on_iso_tile(&mut self, d: TileDone, alloc: Option<Allocation>, store: Option<&Store>) -> Task<ViewerMsg> {
        self.isot.inflight.remove(&d.key);
        let mut tasks = Vec::new();
        if self.isot.inflight.is_empty() {
            let d = self.isot.drag.get_or_insert_with(|| {
                let v: Vec<f32> = std::env::var("OMT_DEBUG_ISO_DRAG").unwrap_or_default().split(',').filter_map(|x| x.trim().parse().ok()).collect();
                match v[..] {
                    [dx, dy, n] => DebugDrag { step: (dx, dy), left: n as u32, ..Default::default() },
                    _ => DebugDrag::default(),
                }
            });
            d.started = true;
        }
        while self.isot.inflight.is_empty() {
            let pan = self.isot.debug_pan.get_or_insert_with(|| {
                std::env::var("OMT_DEBUG_ISO_PAN")
                    .unwrap_or_default()
                    .split(';')
                    .filter_map(|p| match p.trim().strip_prefix('*') {
                        // Zoom steps ride along as (NaN, factor).
                        Some(f) => Some((f32::NAN, f.parse().ok()?)),
                        None => p.split_once(',').and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?))),
                    })
                    .collect()
            });
            let next = pan.pop_front();
            if let Some(t) = self.isot.since.take() {
                eprintln!("iso: view settled in {:.0} ms ({} tiles)", t.elapsed().as_secs_f64() * 1000.0, self.isot.tiles.len());
            }
            let Some((dx, dy)) = next else { break };
            self.isot.since = Some(std::time::Instant::now());
            if dx.is_nan() {
                self.v3.iso_cam.scale = (self.v3.iso_cam.scale * dy).clamp(0.25, 16.0);
            } else {
                self.v3.iso_cam.pan(dx, dy);
            }
            self.sync_iso();
            tasks.push(self.iso_pump(store));
        }
        if d.epoch == self.isot.epoch && self.v3.iso.is_some() {
            // Data that arrived while this tile rendered: render it again.
            let loaded = self.ctl.map.as_ref().is_some_and(|m| d.missing.iter().any(|k| m.tiles.contains_key(k)));
            // Data that arrived meanwhile, or a quick flat pass: redo it.
            if loaded || self.isot.quick.contains(&d.key) {
                self.isot.stale.insert(d.key);
            }
            self.isot.tiles.insert(d.key, IsoTile { handle: d.handle, _alloc: alloc, missing: d.missing });
        }
        tasks.push(self.iso_pump(store));
        Task::batch(tasks)
    }
}

/// One tile render, run off the UI thread.
struct TileJob {
    spec: TileSpec,
    snap: Arc<Snapshot>,
    kinds: Arc<Mutex<HashMap<String, Kind>>>,
    textured: bool,
    sky: [u8; 3],
    level: u8,
    src: omt_render::MapSource,
}

impl TileJob {
    fn run(self) -> (Handle, Vec<TileKey>) {
        let t0 = std::time::Instant::now();
        let out = self.run_inner();
        if std::env::var_os("OMT_DEBUG_ISO_DRAG").is_some() {
            eprintln!("iso job {:.1} ms", t0.elapsed().as_secs_f64() * 1000.0);
        }
        out
    }

    fn run_inner(mut self) -> (Handle, Vec<TileKey>) {
        // Only the heights under this tile bound which columns reach it:
        // the box from the global range holds every candidate column, so
        // the range of the map tiles under it is exact (and usually far
        // tighter — fewer cells to sample).
        if let Some((low, high)) = self.snap.range_in(self.spec.grid_box()) {
            self.spec.low = low;
            self.spec.high = high;
        }
        let pack = if self.textured { crate::textures::current() } else { None };
        // Per-tile kind table, interned from the shared look cache (the
        // grid builder caches ids per row, so this lock is rare).
        let local: Mutex<(Vec<Kind>, HashMap<String, u16>)> = Mutex::new((Vec::new(), HashMap::new()));
        let textured = self.textured;
        let intern = |n: &str| -> u16 {
            let mut l = local.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(&id) = l.1.get(n) {
                return id;
            }
            let kind = self.kinds.lock().unwrap_or_else(|e| e.into_inner()).entry(n.to_string()).or_insert_with(|| crate::viewer_blocks::kind_of(n, pack.as_deref())).clone();
            l.0.push(kind);
            let id = (l.0.len() - 1) as u16;
            l.1.insert(n.to_string(), id);
            id
        };
        let mut grid = self.snap.iso_grid(&self.spec, if textured { Some(&intern) } else { None });
        grid.kinds = local.into_inner().unwrap_or_else(|e| e.into_inner()).0;
        let t_grid = std::time::Instant::now();
        let img = paint_tile(&self.spec, &grid, self.sky, textured);
        if std::env::var_os("OMT_DEBUG_ISO_DRAG").is_some() {
            eprintln!("iso grid n={} range {:.0}..{:.0} paint {:.1} ms", grid.n, self.spec.low, self.spec.high, t_grid.elapsed().as_secs_f64() * 1000.0);
        }
        // Map tiles under this one that exist but were not loaded yet.
        let (x0, z0, x1, z1) = self.spec.grid_box();
        let (tx0, tz0, tx1, tz1) = crate::mapdata::tile_range(self.level, x0, z0, x1, z1);
        let mut missing = Vec::new();
        for tz in tz0..=tz1 {
            for tx in tx0..=tx1 {
                let k = TileKey::new(self.level, tx, tz);
                if self.snap.has(&k) {
                    continue;
                }
                let (a, b, c, d) = k.chunk_rect();
                if self.src.chunks().any_in_rect(a, b, c, d) {
                    missing.push(k);
                }
            }
        }
        let size = self.spec.size as u32;
        (Handle::from_rgba(size, size, img), missing)
    }
}

#[cfg(test)]
#[path = "viewer_iso_tests.rs"]
mod tests;
