//! Level-of-detail area rendering — the engine behind on-demand map tiles
//! (GUI) and streamed whole-world PNGs (CLI).
//!
//! An area is rendered in **bands of chunk rows**: each band is read,
//! decoded, rendered, shaded, reduced to output pixels and handed to a sink,
//! then dropped. Peak memory is bounded by the band size, never by the area
//! or world size.
//!
//! Detail model (`level` L, `stride` S):
//! * `S == 1` — every chunk is read. One output pixel covers 2^L × 2^L
//!   blocks and is the box average of the full-detail (L0) pixels, so
//!   L0 output is pixel-identical to a whole-world render.
//! * `S > 1` — sampled: one representative chunk per S × S chunk cell (the
//!   present chunk nearest the cell center), one pixel per cell (its
//!   average color, relief-shaded against the northern cell). Reads drop by
//!   S² — the "far zoom / low quality" mode.

use std::collections::HashMap;

use crate::palette::Palette;
use crate::render::{ChunkRender, NO_SURFACE, RenderOptions, shade};
use crate::source::MapSource;

/// Chunks per tile edge at level 0 (= one region).
pub const TILE_CHUNKS: i32 = 32;
/// Nominal tile image edge in pixels (full detail).
pub const TILE_PX: u32 = 512;
/// Coarsest level (a level-L tile spans `32 << L` chunks).
pub const MAX_LEVEL: u8 = 12;
/// Accumulator pixels per band (caps band memory for very wide areas:
/// ~16 bytes per pixel while a band is reduced).
const MAX_BAND_PX: usize = 8 << 20;

/// A tile of the LOD pyramid. Level-L tiles span `32 << L` chunks per edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub level: u8,
    pub tx: i32,
    pub tz: i32,
}

impl TileKey {
    pub fn new(level: u8, tx: i32, tz: i32) -> Self {
        Self { level, tx, tz }
    }

    /// Chunks per tile edge.
    pub fn span(self) -> i32 {
        TILE_CHUNKS << self.level
    }

    /// Blocks per tile edge.
    pub fn span_blocks(self) -> i64 {
        self.span() as i64 * 16
    }

    /// Inclusive chunk rectangle `(x0, z0, x1, z1)`.
    pub fn chunk_rect(self) -> (i32, i32, i32, i32) {
        let s = self.span();
        (self.tx * s, self.tz * s, self.tx * s + s - 1, self.tz * s + s - 1)
    }

    /// The tile of `level` containing a chunk.
    pub fn containing(level: u8, cx: i32, cz: i32) -> Self {
        let s = TILE_CHUNKS << level;
        Self { level, tx: cx.div_euclid(s), tz: cz.div_euclid(s) }
    }

    /// The covering tile one level coarser.
    pub fn parent(self) -> Option<Self> {
        (self.level < MAX_LEVEL).then(|| Self { level: self.level + 1, tx: self.tx.div_euclid(2), tz: self.tz.div_euclid(2) })
    }
}

/// Chunk sampling stride for a level under a per-tile chunk-read budget:
/// the smallest power of two keeping `(span / stride)²` reads within budget
/// (`None` = unlimited, always full detail).
pub fn stride_for(level: u8, budget: Option<u64>) -> i32 {
    let span = TILE_CHUNKS << level;
    let Some(budget) = budget else { return 1 };
    let mut stride = 1i32;
    while stride < span {
        let n = (span / stride) as u64;
        if n * n <= budget.max(1) {
            break;
        }
        stride *= 2;
    }
    stride
}

/// Sampling stride for a whole LOD level, adapted to the chunks that
/// actually exist (sparse worlds — exploration trails, islands — are mostly
/// empty space). ONE stride per level keeps every tile at a zoom equally
/// sharp (per-tile strides left visible quality seams):
/// * every tile's present chunks fit the budget → stride 1 (exact);
/// * otherwise the finest stride (≥ the level's nominal pixel size) at
///   which the densest tile's **occupied** cells fit the budget.
///
/// Never coarser than [`stride_for`] (the dense worst case). Deterministic
/// for a given chunk set (safe as a cache key).
pub fn level_stride(chunks: &omt_world::ChunkSet, level: u8, budget: Option<u64>) -> i32 {
    use std::collections::{HashMap, HashSet};
    let Some(budget) = budget else { return 1 };
    let span = TILE_CHUNKS << level;
    let mut per_tile: HashMap<(i32, i32), u64> = HashMap::new();
    for (x, z) in chunks.iter() {
        *per_tile.entry((x.div_euclid(span), z.div_euclid(span))).or_default() += 1;
    }
    if per_tile.values().all(|&n| n <= budget) {
        return 1;
    }
    let dense = stride_for(level, Some(budget));
    let mut stride = (span / TILE_PX as i32).max(2);
    while stride < dense {
        let cells: HashSet<(i32, i32)> = chunks.iter().map(|(x, z)| (x.div_euclid(stride), z.div_euclid(stride))).collect();
        let per = span / stride;
        let mut counts: HashMap<(i32, i32), u64> = HashMap::new();
        let mut ok = true;
        for (cx, cz) in cells {
            let n = counts.entry((cx.div_euclid(per), cz.div_euclid(per))).or_default();
            *n += 1;
            if *n > budget {
                ok = false;
                break;
            }
        }
        if ok {
            return stride;
        }
        stride *= 2;
    }
    dense
}

/// What to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AreaSpec {
    /// Top-left chunk.
    pub x0: i32,
    pub z0: i32,
    /// Size in chunks.
    pub w: i32,
    pub h: i32,
    /// One pixel = 2^level blocks (`stride == 1` only).
    pub level: u8,
    /// Chunk sampling stride (1 = read every chunk).
    pub stride: i32,
    /// Also read the chunk row (or cell row) north of the area so the first
    /// row's height shading matches a render of the surrounding world.
    pub north_row: bool,
}

impl AreaSpec {
    /// Chunk rows per output pixel row (`stride == 1`, level > 4).
    fn chunks_per_px(&self) -> i32 {
        if self.level > 4 { 1 << (self.level - 4) } else { 1 }
    }

    /// Output pixel dimensions `(width, height)`.
    pub fn px_size(&self) -> (usize, usize) {
        let ceil = |a: i32, b: i32| ((a + b - 1) / b) as usize;
        if self.stride > 1 {
            (ceil(self.w, self.stride), ceil(self.h, self.stride))
        } else if self.level <= 4 {
            ((self.w as usize * 16) >> self.level, (self.h as usize * 16) >> self.level)
        } else {
            let c = self.chunks_per_px();
            (ceil(self.w, c), ceil(self.h, c))
        }
    }

    /// Blocks per output pixel edge.
    pub fn blocks_per_px(&self) -> i64 {
        if self.stride > 1 { self.stride as i64 * 16 } else { 1i64 << self.level }
    }
}

/// A finished band of output rows `[y0, y0 + rows)`.
pub struct Band<'a> {
    pub y0: usize,
    pub rows: usize,
    pub width: usize,
    pub rgba: &'a [u8],
    /// Surface name id per pixel ([`NO_SURFACE`] = none) into the caller's
    /// [`NameTable`].
    pub surface: &'a [u16],
    /// Surface height per pixel (`i16::MIN` = none).
    pub heights: &'a [i16],
    /// Surface biome per pixel (name id, [`NO_SURFACE`] = unknown).
    pub biomes: &'a [u16],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AreaStats {
    /// Chunks read and rendered.
    pub chunks: usize,
    /// Chunks that exist but failed to decode.
    pub errors: usize,
}

/// Surface block names shared across an area's pixels.
#[derive(Debug, Clone, Default)]
pub struct NameTable {
    pub names: Vec<String>,
    ids: HashMap<String, u16>,
}

impl NameTable {
    pub fn id(&mut self, name: &str) -> u16 {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        if self.names.len() >= NO_SURFACE as usize {
            return NO_SURFACE;
        }
        let id = self.names.len() as u16;
        self.names.push(name.to_string());
        self.ids.insert(name.to_string(), id);
        id
    }
}

/// Render an area band by band. `band_chunks` bounds the chunks read per
/// band (memory ceiling). Returns `None` when `cancel` fired.
#[allow(clippy::too_many_arguments)]
pub fn render_area(
    src: &MapSource,
    spec: AreaSpec,
    palette: &Palette,
    opts: &RenderOptions,
    band_chunks: usize,
    names: &mut NameTable,
    cancel: &(dyn Fn() -> bool + Sync),
    sink: &mut dyn FnMut(Band<'_>),
) -> Option<AreaStats> {
    if spec.stride > 1 {
        render_sampled(src, spec, palette, opts, band_chunks, names, cancel, sink)
    } else {
        render_full(src, spec, palette, opts, band_chunks, names, cancel, sink)
    }
}

fn biome_of(r: &ChunkRender, col: usize, names: &mut NameTable) -> u16 {
    r.biome_name(col).map_or(NO_SURFACE, |n| names.id(n))
}

fn surface_of(r: &ChunkRender, col: usize, names: &mut NameTable) -> (u16, i16) {
    let h = r.heights[col];
    let id = r.surface_name(col).map_or(NO_SURFACE, |n| names.id(n));
    let h = if h == i32::MIN { i16::MIN } else { h.clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16 };
    (id, h)
}

/// Present chunks of a rect grouped by row (`cz` → sorted `cx`).
fn rows_of(src: &MapSource, x0: i32, z0: i32, x1: i32, z1: i32) -> HashMap<i32, Vec<i32>> {
    let mut rows: HashMap<i32, Vec<i32>> = HashMap::new();
    for (x, z) in src.chunks().in_rect(x0, z0, x1, z1) {
        rows.entry(z).or_default().push(x);
    }
    for v in rows.values_mut() {
        v.sort_unstable();
    }
    rows
}

#[allow(clippy::too_many_arguments)]
fn render_full(
    src: &MapSource,
    spec: AreaSpec,
    palette: &Palette,
    opts: &RenderOptions,
    band_chunks: usize,
    names: &mut NameTable,
    cancel: &(dyn Fn() -> bool + Sync),
    sink: &mut dyn FnMut(Band<'_>),
) -> Option<AreaStats> {
    let (pw, ph) = spec.px_size();
    let cpp = spec.chunks_per_px();
    let level = spec.level.min(4) as usize;
    let ppc = 16usize >> level; // px per chunk edge (level ≤ 4)
    let x1 = spec.x0 + spec.w - 1;
    let z1 = spec.z0 + spec.h - 1;
    let rows = rows_of(src, spec.x0, spec.z0 - i32::from(spec.north_row), x1, z1);
    let mut stats = AreaStats::default();

    // North neighbor heights for the first row.
    let mut prev: HashMap<i32, [i32; 256]> = HashMap::new();
    if spec.north_row
        && let Some(xs) = rows.get(&(spec.z0 - 1))
    {
        let coords: Vec<(i32, i32)> = xs.iter().map(|&x| (x, spec.z0 - 1)).collect();
        let out = src.render(&coords, palette, opts, cancel)?;
        for r in out.renders {
            prev.insert(r.x, r.heights);
        }
    }

    // Band = whole pixel-row groups (cpp chunk rows each), grown until it
    // holds `band_chunks` PRESENT chunks (sparse worlds: wide rows with few
    // chunks still batch well) or its accumulator reaches MAX_BAND_PX.
    let px_rows_per_group = if spec.level <= 4 { ppc } else { 1 };
    let mut cz = spec.z0;
    while cz <= z1 {
        if cancel() {
            return None;
        }
        let bz0 = cz;
        let mut bz1 = bz0 - 1;
        let (mut present, mut px_rows) = (0usize, 0usize);
        while bz1 < z1 {
            let g1 = (bz1 + cpp).min(z1);
            present += (bz1 + 1..=g1).map(|z| rows.get(&z).map_or(0, Vec::len)).sum::<usize>();
            px_rows += px_rows_per_group;
            bz1 = g1;
            if present >= band_chunks || (px_rows + px_rows_per_group) * pw > MAX_BAND_PX {
                break;
            }
        }
        let mut coords = Vec::new();
        for z in bz0..=bz1 {
            if let Some(xs) = rows.get(&z) {
                coords.extend(xs.iter().map(|&x| (x, z)));
            }
        }
        let out = src.render(&coords, palette, opts, cancel)?;
        stats.chunks += out.renders.len();
        stats.errors += out.errors;
        let mut map: HashMap<(i32, i32), ChunkRender> = out.renders.into_iter().map(|r| ((r.x, r.z), r)).collect();
        // Height shading against the northern neighbor (unshaded heights).
        let norths: HashMap<(i32, i32), [i32; 256]> = map
            .keys()
            .filter_map(|&(x, z)| {
                let n = if z == bz0 { prev.get(&x).copied() } else { map.get(&(x, z - 1)).map(|r| r.heights) };
                n.map(|h| ((x, z), h))
            })
            .collect();
        if opts.relief {
            for (k, r) in map.iter_mut() {
                shade(r, norths.get(k));
            }
        }

        // Reduce into this band's pixel rows.
        let py0 = if spec.level <= 4 { ((bz0 - spec.z0) as usize) * ppc } else { ((bz0 - spec.z0) / cpp) as usize };
        let py1 = if spec.level <= 4 { ((bz1 - spec.z0 + 1) as usize) * ppc } else { ((bz1 - spec.z0) / cpp) as usize + 1 };
        let brow = py1.min(ph) - py0;
        let mut acc = vec![0u32; pw * brow * 3];
        let mut cnt = vec![0u32; pw * brow];
        let mut surf = vec![NO_SURFACE; pw * brow];
        let mut hts = vec![i16::MIN; pw * brow];
        let mut bio = vec![NO_SURFACE; pw * brow];
        let mut keys: Vec<(i32, i32)> = map.keys().copied().collect();
        keys.sort_unstable_by_key(|&(x, z)| (z, x));
        for (x, z) in keys {
            let r = &map[&(x, z)];
            if spec.level <= 4 {
                let ox = (x - spec.x0) as usize * ppc;
                let oz = (z - bz0) as usize * ppc;
                let s = 1usize << level;
                for lz in 0..16 {
                    for lx in 0..16 {
                        let p = (oz + (lz >> level)) * pw + ox + (lx >> level);
                        let i = (lz * 16 + lx) * 4;
                        if r.rgba[i + 3] != 0 {
                            acc[p * 3] += r.rgba[i] as u32;
                            acc[p * 3 + 1] += r.rgba[i + 1] as u32;
                            acc[p * 3 + 2] += r.rgba[i + 2] as u32;
                            cnt[p] += 1;
                        }
                        if lz % s == 0 && lx % s == 0 {
                            let (id, h) = surface_of(r, lz * 16 + lx, names);
                            surf[p] = id;
                            hts[p] = h;
                            bio[p] = biome_of(r, lz * 16 + lx, names);
                        }
                    }
                }
            } else {
                let p = ((z - spec.z0) / cpp) as usize - py0;
                let p = p * pw + ((x - spec.x0) / cpp) as usize;
                for px in r.rgba.as_chunks::<4>().0 {
                    if px[3] != 0 {
                        acc[p * 3] += px[0] as u32;
                        acc[p * 3 + 1] += px[1] as u32;
                        acc[p * 3 + 2] += px[2] as u32;
                        cnt[p] += 1;
                    }
                }
                if surf[p] == NO_SURFACE {
                    let (id, h) = surface_of(r, 0, names);
                    surf[p] = id;
                    hts[p] = h;
                    bio[p] = biome_of(r, 0, names);
                }
            }
        }
        prev = map.into_iter().filter(|((_, z), _)| *z == bz1).map(|((x, _), r)| (x, r.heights)).collect();
        let mut rgba = vec![0u8; pw * brow * 4];
        for p in 0..pw * brow {
            let n = cnt[p];
            for c in 0..3 {
                // Rounded mean of the opaque samples (none → stays clear).
                if let Some(v) = (acc[p * 3 + c] + n / 2).checked_div(n) {
                    rgba[p * 4 + c] = v as u8;
                }
            }
            if n > 0 {
                rgba[p * 4 + 3] = 255;
            }
        }
        sink(Band { y0: py0, rows: brow, width: pw, rgba: &rgba, surface: &surf, heights: &hts, biomes: &bio });
        cz = bz1 + 1;
    }
    Some(stats)
}

#[allow(clippy::too_many_arguments)]
fn render_sampled(
    src: &MapSource,
    spec: AreaSpec,
    palette: &Palette,
    opts: &RenderOptions,
    band_chunks: usize,
    names: &mut NameTable,
    cancel: &(dyn Fn() -> bool + Sync),
    sink: &mut dyn FnMut(Band<'_>),
) -> Option<AreaStats> {
    let s = spec.stride;
    let (pw, ph) = spec.px_size();
    let x1 = spec.x0 + spec.w - 1;
    let z1 = spec.z0 + spec.h - 1;
    let north = i32::from(spec.north_row);
    // Representative chunk per cell: nearest the cell center (ties → raster order).
    let mut reps: HashMap<(i32, i32), ((i32, i32), i64)> = HashMap::new();
    for (x, z) in src.chunks().in_rect(spec.x0, spec.z0 - s * north, x1, z1) {
        let cell = ((x - spec.x0).div_euclid(s), (z - spec.z0).div_euclid(s));
        let (cx, cz) = (cell.0 * s + spec.x0 + s / 2, cell.1 * s + spec.z0 + s / 2);
        let d = ((x - cx) as i64).pow(2) + ((z - cz) as i64).pow(2);
        let better = reps.get(&cell).is_none_or(|&((bx, bz), bd)| d < bd || (d == bd && (z, x) < (bz, bx)));
        if better {
            reps.insert(cell, ((x, z), d));
        }
    }
    let mut stats = AreaStats::default();
    let cell_px = |r: &ChunkRender| -> ([u8; 4], Option<i32>) { (r.average().unwrap_or([0; 4]), r.mean_height()) };

    // Mean heights of the northern cell row (relief shading of row 0).
    let mut prev: HashMap<i32, i32> = HashMap::new();
    if spec.north_row {
        let coords: Vec<(i32, i32)> = reps.iter().filter(|(c, _)| c.1 == -1).map(|(_, v)| v.0).collect();
        let out = src.render(&coords, palette, opts, cancel)?;
        for r in &out.renders {
            if let Some(h) = r.mean_height() {
                prev.insert((r.x - spec.x0).div_euclid(s), h);
            }
        }
    }

    // Cell rows per band: grown until `band_chunks` representatives (or the
    // pixel cap) — see `render_full`.
    let mut per_row = vec![0usize; ph];
    for &(cx, cz) in reps.keys() {
        if cz >= 0 && (cz as usize) < ph && cx >= 0 && (cx as usize) < pw {
            per_row[cz as usize] += 1;
        }
    }
    let mut row = 0usize;
    while row < ph {
        if cancel() {
            return None;
        }
        let mut r1 = row;
        let mut present = 0usize;
        while r1 < ph {
            present += per_row[r1];
            r1 += 1;
            if present >= band_chunks || (r1 - row + 1) * pw > MAX_BAND_PX {
                break;
            }
        }
        let mut coords = Vec::new();
        let mut owner: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
        for cz in row as i32..r1 as i32 {
            for cx in 0..pw as i32 {
                if let Some(&(c, _)) = reps.get(&(cx, cz)) {
                    coords.push(c);
                    owner.insert(c, (cx, cz));
                }
            }
        }
        let out = src.render(&coords, palette, opts, cancel)?;
        stats.chunks += out.renders.len();
        stats.errors += out.errors;
        let brow = r1 - row;
        let mut rgba = vec![0u8; pw * brow * 4];
        let mut surf = vec![NO_SURFACE; pw * brow];
        let mut hts = vec![i16::MIN; pw * brow];
        let mut bio = vec![NO_SURFACE; pw * brow];
        let mut mean: HashMap<(i32, i32), i32> = HashMap::new();
        let mut cells: Vec<((i32, i32), [u8; 4], &ChunkRender)> = Vec::new();
        for r in &out.renders {
            let Some(&cell) = owner.get(&(r.x, r.z)) else { continue };
            let (c, h) = cell_px(r);
            if let Some(h) = h {
                mean.insert(cell, h);
            }
            cells.push((cell, c, r));
        }
        cells.sort_unstable_by_key(|&(c, _, _)| (c.1, c.0));
        for (cell, mut c, r) in cells {
            if c[3] == 0 {
                continue;
            }
            let north_h = if cell.1 as usize == row { prev.get(&cell.0).copied() } else { mean.get(&(cell.0, cell.1 - 1)).copied() };
            if let (true, Some(h), Some(n)) = (opts.relief, mean.get(&cell), north_h) {
                let f = (1.0 + (h - n) as f32 * 0.015).clamp(0.82, 1.12);
                for v in c.iter_mut().take(3) {
                    *v = (*v as f32 * f).round().clamp(0.0, 255.0) as u8;
                }
            }
            let p = (cell.1 as usize - row) * pw + cell.0 as usize;
            rgba[p * 4..p * 4 + 4].copy_from_slice(&c);
            // Identify: the surface nearest the chunk center.
            let col = [8 * 16 + 8, 0].into_iter().find(|&i| r.surface[i] != NO_SURFACE).unwrap_or(8 * 16 + 8);
            let (id, h) = surface_of(r, col, names);
            surf[p] = id;
            hts[p] = h;
            bio[p] = biome_of(r, col, names);
        }
        prev = mean.iter().filter(|((_, z), _)| *z as usize == r1 - 1).map(|(&(x, _), &h)| (x, h)).collect();
        sink(Band { y0: row, rows: brow, width: pw, rgba: &rgba, surface: &surf, heights: &hts, biomes: &bio });
        row = r1;
    }
    Some(stats)
}

/// One rendered tile of the LOD pyramid.
#[derive(Debug, Clone)]
pub struct TileImage {
    pub key: TileKey,
    pub stride: i32,
    /// Image edge in pixels (0 for an empty tile).
    pub px: u32,
    pub rgba: Vec<u8>,
    pub surface: Vec<u16>,
    pub heights: Vec<i16>,
    /// Surface biome per pixel (id into `names`, [`NO_SURFACE`] = unknown).
    pub biomes: Vec<u16>,
    pub names: Vec<String>,
    pub stats: AreaStats,
    pub millis: u64,
}

impl TileImage {
    /// A tile with no chunks (nothing to draw, nothing read).
    pub fn empty(key: TileKey, stride: i32) -> Self {
        Self { key, stride, px: 0, rgba: Vec::new(), surface: Vec::new(), heights: Vec::new(), biomes: Vec::new(), names: Vec::new(), stats: AreaStats::default(), millis: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.px == 0
    }

    /// Heap bytes held (cache accounting).
    pub fn heap_bytes(&self) -> usize {
        self.rgba.capacity() + self.surface.capacity() * 2 + self.heights.capacity() * 2 + self.biomes.capacity() * 2 + self.names.iter().map(|n| n.capacity() + 24).sum::<usize>() + 128
    }

    /// Blocks per pixel edge.
    pub fn blocks_per_px(&self) -> i64 {
        if self.px == 0 { 1 } else { self.key.span_blocks() / self.px as i64 }
    }

    /// Surface block + height at a block position. Exact at full detail
    /// (level 0); a representative sample of the pixel otherwise.
    pub fn identify(&self, bx: i64, bz: i64) -> Option<(&str, i32)> {
        if self.px == 0 {
            return None;
        }
        let bpp = self.blocks_per_px();
        let (x0, z0, _, _) = self.key.chunk_rect();
        let lx = (bx - x0 as i64 * 16).div_euclid(bpp);
        let lz = (bz - z0 as i64 * 16).div_euclid(bpp);
        if lx < 0 || lz < 0 || lx >= self.px as i64 || lz >= self.px as i64 {
            return None;
        }
        let i = (lz * self.px as i64 + lx) as usize;
        let h = *self.heights.get(i)?;
        if h == i16::MIN {
            return None;
        }
        Some((self.names.get(*self.surface.get(i)? as usize)?.as_str(), h as i32))
    }
}

/// Render one tile. `budget` = max chunk reads per tile (`None` = full
/// detail; the stride adapts to the chunks present — see [`level_stride`]).
/// Returns `None` when cancelled.
pub fn build_tile(
    src: &MapSource,
    key: TileKey,
    budget: Option<u64>,
    palette: &Palette,
    opts: &RenderOptions,
    cancel: &(dyn Fn() -> bool + Sync),
) -> Option<TileImage> {
    let started = std::time::Instant::now();
    let stride = src.level_stride(key.level, budget);
    let (x0, z0, x1, z1) = key.chunk_rect();
    if !src.chunks().any_in_rect(x0, z0, x1, z1) {
        return Some(TileImage::empty(key, stride));
    }
    let spec = AreaSpec { x0, z0, w: key.span(), h: key.span(), level: key.level, stride, north_row: true };
    let (pw, ph) = spec.px_size();
    let mut rgba = vec![0u8; pw * ph * 4];
    let mut surface = vec![NO_SURFACE; pw * ph];
    let mut heights = vec![i16::MIN; pw * ph];
    let mut biomes = vec![NO_SURFACE; pw * ph];
    let mut names = NameTable::default();
    let stats = render_area(src, spec, palette, opts, 4096, &mut names, cancel, &mut |b: Band<'_>| {
        let n = b.rows * b.width;
        let o = b.y0 * b.width;
        rgba[o * 4..(o + n) * 4].copy_from_slice(&b.rgba[..n * 4]);
        surface[o..o + n].copy_from_slice(&b.surface[..n]);
        heights[o..o + n].copy_from_slice(&b.heights[..n]);
        biomes[o..o + n].copy_from_slice(&b.biomes[..n]);
    })?;
    Some(TileImage {
        key,
        stride,
        px: pw as u32,
        rgba,
        surface,
        heights,
        biomes,
        names: names.names,
        stats,
        millis: started.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_geometry() {
        let k = TileKey::new(0, -1, 2);
        assert_eq!(k.span(), 32);
        assert_eq!(k.chunk_rect(), (-32, 64, -1, 95));
        assert_eq!(TileKey::containing(0, -1, 64), k);
        assert_eq!(TileKey::containing(2, -1, 64), TileKey::new(2, -1, 0));
        assert_eq!(k.parent(), Some(TileKey::new(1, -1, 1)));
    }

    #[test]
    fn stride_respects_budget() {
        assert_eq!(stride_for(0, None), 1);
        assert_eq!(stride_for(3, None), 1);
        assert_eq!(stride_for(0, Some(1024)), 1); // 32² = 1024
        assert_eq!(stride_for(1, Some(4096)), 1); // 64²
        assert_eq!(stride_for(2, Some(4096)), 2); // 128² → 64²
        assert_eq!(stride_for(4, Some(4096)), 8); // 512² → 64²
        assert_eq!(stride_for(2, Some(16384)), 1);
    }

    #[test]
    fn level_stride_is_uniform_and_adapts_to_density() {
        // Sparse: a 40-chunk trail fits any budget → exact at every level.
        let trail: omt_world::ChunkSet = (0..40).map(|i| (i * 7, i * 5)).collect();
        assert_eq!(level_stride(&trail, 4, Some(1024)), 1);
        // Dense 512×512 world: equals the dense worst case.
        let dense: omt_world::ChunkSet = (0..512).flat_map(|x| (0..512).map(move |z| (x, z))).collect();
        assert_eq!(level_stride(&dense, 4, Some(4096)), stride_for(4, Some(4096)));
        // A filled 200×200 block needs a finer stride than the dense case,
        // and the densest tile decides for the whole level.
        let mut mixed: omt_world::ChunkSet = (0..200).flat_map(|x| (0..200).map(move |z| (x, z))).collect();
        mixed.insert(5000, 5000);
        let s = level_stride(&mixed, 4, Some(4096));
        assert!(s > 1 && s < stride_for(4, Some(4096)), "stride {s}");
        assert_eq!(level_stride(&dense, 4, None), 1);
    }

    #[test]
    fn px_sizes() {
        let base = AreaSpec { x0: 0, z0: 0, w: 32, h: 32, level: 0, stride: 1, north_row: false };
        assert_eq!(base.px_size(), (512, 512));
        assert_eq!(AreaSpec { level: 2, w: 128, h: 128, ..base }.px_size(), (512, 512));
        assert_eq!(AreaSpec { level: 6, w: 2048, h: 2048, ..base }.px_size(), (512, 512));
        assert_eq!(AreaSpec { level: 2, w: 128, h: 128, stride: 2, ..base }.px_size(), (64, 64));
        assert_eq!(AreaSpec { level: 0, w: 5, h: 3, stride: 2, ..base }.px_size(), (3, 2));
        assert_eq!(AreaSpec { level: 1, ..base }.blocks_per_px(), 2);
        assert_eq!(AreaSpec { stride: 4, ..base }.blocks_per_px(), 64);
    }
}
