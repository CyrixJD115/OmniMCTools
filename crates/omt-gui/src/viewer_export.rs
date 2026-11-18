//! Map Viewer export: compose the current view from the loaded (styled)
//! tiles — at most 2× the screen resolution, never re-reading the world —
//! and write it as PNG.

use omt_render::TileKey;

use crate::mapdata::MapController;

/// Largest exported edge in pixels.
const MAX_EDGE: f32 = 8192.0;

/// The visible area as RGBA at the display level's native resolution
/// (one pixel per tile pixel, capped). `None` when nothing is loaded.
pub fn compose(ctl: &MapController) -> Option<(u32, u32, Vec<u8>)> {
    let map = ctl.map.as_ref()?;
    let level = map.level;
    let (bx0, bz0, bx1, bz1) = ctl.camera.visible_blocks(ctl.view);
    // Blocks per output pixel: a level-L tile shows 2^L blocks per pixel
    // at full detail; coarsened when the image would exceed the cap.
    let native = (1u64 << level) as f64;
    let mut bpp = native;
    while ((bx1 - bx0) / bpp) as f32 > MAX_EDGE || ((bz1 - bz0) / bpp) as f32 > MAX_EDGE {
        bpp *= 2.0;
    }
    let w = (((bx1 - bx0) / bpp).ceil() as u32).max(1);
    let h = (((bz1 - bz0) / bpp).ceil() as u32).max(1);
    let mut out = vec![0u8; (w * h * 4) as usize];
    // Decode each needed tile once.
    let mut cache: std::collections::HashMap<TileKey, Option<(u32, Vec<u8>)>> = std::collections::HashMap::new();
    let mut any = false;
    for py in 0..h {
        for px in 0..w {
            let bx = bx0 + (px as f64 + 0.5) * bpp;
            let bz = bz0 + (py as f64 + 0.5) * bpp;
            let (cx, cz) = ((bx.floor() as i64).div_euclid(16) as i32, (bz.floor() as i64).div_euclid(16) as i32);
            // Finest loaded level first.
            for l in level..=omt_render::MAX_LEVEL {
                let key = TileKey::containing(l, cx, cz);
                let Some(tile) = map.tiles.get(&key) else { continue };
                let entry = cache.entry(key).or_insert_with(|| tile.pixels());
                let Some((tpx, rgba)) = entry else { break };
                let (x0, z0, _, _) = key.chunk_rect();
                let tbpp = key.span_blocks() as f64 / *tpx as f64;
                let lx = ((bx - x0 as f64 * 16.0) / tbpp).floor() as i64;
                let lz = ((bz - z0 as f64 * 16.0) / tbpp).floor() as i64;
                if lx < 0 || lz < 0 || lx >= *tpx as i64 || lz >= *tpx as i64 {
                    break;
                }
                let s = ((lz * *tpx as i64 + lx) * 4) as usize;
                let o = ((py * w + px) * 4) as usize;
                out[o..o + 4].copy_from_slice(&rgba[s..s + 4]);
                any = true;
                break;
            }
        }
    }
    any.then_some((w, h, out))
}

pub fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgba).map_err(|e| e.to_string())
}

// ------------------------------------------------------------ area export
//
// uNmINeD's "Image export": any area (the view, the whole world, or a
// block rectangle) at a chosen zoom, rendered fresh from the world in
// style-matched map tiles and streamed into the PNG one tile row at a
// time (memory stays at one row of tiles, whatever the image size).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use omt_render::style::StyleParams;
use omt_render::{MapSource, Palette, RenderOptions, TILE_CHUNKS, TILE_PX};

/// Which area to export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Area {
    #[default]
    View,
    World,
    Custom,
}

impl Area {
    pub const ALL: [Area; 3] = [Self::View, Self::World, Self::Custom];
}

impl std::fmt::Display for Area {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::View => "Visible view",
            Self::World => "Entire world",
            Self::Custom => "Custom area",
        })
    }
}

/// Export zoom: `-4..=-1` = 16/8/4/2 blocks per pixel, `0` = 1:1, `1`/`2`
/// = 2/4 pixels per block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zoom(pub i8);

impl Zoom {
    pub const ALL: [Zoom; 7] = [Zoom(-4), Zoom(-3), Zoom(-2), Zoom(-1), Zoom(0), Zoom(1), Zoom(2)];
    /// Render level (blocks per pixel = 2^level) and integer upscale.
    pub fn level_scale(self) -> (u8, u32) {
        if self.0 < 0 { ((-self.0) as u8, 1) } else { (0, 1 << self.0) }
    }
}

impl std::fmt::Display for Zoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            z if z < 0 => write!(f, "1:{} ({} blocks per pixel)", 1 << -z, 1 << -z),
            0 => f.write_str("1:1 (1 pixel per block)"),
            z => write!(f, "{}:1 ({} pixels per block)", 1 << z, 1 << z),
        }
    }
}

/// Export form state.
#[derive(Debug)]
pub struct ExportState {
    pub area: Area,
    pub zoom: Zoom,
    pub min: (String, String),
    pub max: (String, String),
    /// Background behind empty pixels: `None` = transparent.
    pub background: String,
    pub trim: bool,
    pub busy: Option<(Arc<AtomicBool>, Arc<AtomicU64>, u64)>,
    pub status: String,
}

impl Default for ExportState {
    fn default() -> Self {
        Self { area: Area::View, zoom: Zoom(0), min: ("-512".into(), "-512".into()), max: ("511".into(), "511".into()), background: String::new(), trim: true, busy: None, status: String::new() }
    }
}

/// A resolved export job.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AreaJob {
    /// Block rectangle (inclusive min, exclusive max).
    pub x0: i64,
    pub z0: i64,
    pub x1: i64,
    pub z1: i64,
    pub level: u8,
    pub scale: u32,
    pub background: Option<[u8; 3]>,
}

impl AreaJob {
    pub fn px_size(&self) -> (u64, u64) {
        let bpp = 1i64 << self.level;
        let w = ((self.x1 - self.x0) as u64).div_ceil(bpp as u64) * self.scale as u64;
        let h = ((self.z1 - self.z0) as u64).div_ceil(bpp as u64) * self.scale as u64;
        (w, h)
    }

    /// Working memory estimate (one row of tiles + PNG row buffers).
    pub fn memory_bytes(&self) -> u64 {
        let (w, _) = self.px_size();
        w * TILE_PX as u64 * 4 * 3
    }
}

/// Largest exported edge (PNG limit is 2^31; keep files sane).
pub const MAX_EXPORT_EDGE: u64 = 65_536;

/// Resolve the form into a job (`Err` = message for the user).
pub fn resolve(st: &ExportState, ctl: &MapController) -> Result<AreaJob, String> {
    let map = ctl.map.as_ref().ok_or("the map is still loading")?;
    let (mut x0, mut z0, mut x1, mut z1) = match st.area {
        Area::View => {
            let (a, b, c, d) = ctl.camera.visible_blocks(ctl.view);
            (a.floor() as i64, b.floor() as i64, c.ceil() as i64, d.ceil() as i64)
        }
        Area::World => {
            let (a, b, c, d) = map.source.chunks().bounds().ok_or("this dimension has no chunks")?;
            (a as i64 * 16, b as i64 * 16, (c as i64 + 1) * 16, (d as i64 + 1) * 16)
        }
        Area::Custom => {
            let p = |s: &str| s.trim().parse::<i64>().map_err(|_| format!("“{s}” is not a block coordinate"));
            let (a, b, c, d) = (p(&st.min.0)?, p(&st.min.1)?, p(&st.max.0)?, p(&st.max.1)?);
            (a.min(c), b.min(d), a.max(c) + 1, b.max(d) + 1)
        }
    };
    if st.trim
        && let Some((a, b, c, d)) = map.source.chunks().bounds()
    {
        x0 = x0.max(a as i64 * 16);
        z0 = z0.max(b as i64 * 16);
        x1 = x1.min((c as i64 + 1) * 16);
        z1 = z1.min((d as i64 + 1) * 16);
    }
    if x1 <= x0 || z1 <= z0 {
        return Err("the area holds no chunks".into());
    }
    let background = match st.background.trim() {
        "" => None,
        s => Some(crate::panelui::parse_color(s).ok_or("background: use #rrggbb or leave empty for transparent")?),
    };
    let (level, scale) = st.zoom.level_scale();
    let job = AreaJob { x0, z0, x1, z1, level, scale, background };
    let (w, h) = job.px_size();
    if w > MAX_EXPORT_EDGE || h > MAX_EXPORT_EDGE {
        return Err(format!("{w}×{h} px is too large (max {MAX_EXPORT_EDGE} per edge) — zoom out"));
    }
    Ok(job)
}

/// How to render an export: the map's options, style and CPU budget.
#[derive(Debug, Clone, Copy)]
pub struct Look {
    pub opts: RenderOptions,
    pub style: StyleParams,
    pub threads: usize,
}

/// Render `job` and stream it into a PNG at `path`. `done` counts tiles.
pub fn export_area(src: &MapSource, look: Look, job: AreaJob, path: &std::path::Path, cancel: &AtomicBool, done: &AtomicU64) -> Result<String, String> {
    let Look { opts, style, threads } = look;
    let (w, h) = job.px_size();
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?.into_stream_writer().map_err(|e| e.to_string())?;
    let span_blocks = (TILE_CHUNKS as i64 * 16) << job.level;
    let bpp = 1i64 << job.level;
    let (tx0, tx1) = (job.x0.div_euclid(span_blocks), (job.x1 - 1).div_euclid(span_blocks));
    let (tz0, tz1) = (job.z0.div_euclid(span_blocks), (job.z1 - 1).div_euclid(span_blocks));
    let row_px = (w / job.scale as u64) as usize;
    let cancelled = || cancel.load(Ordering::Relaxed);
    for tz in tz0..=tz1 {
        if cancelled() {
            return Err("cancelled".into());
        }
        // Render this row of tiles (in parallel on the render pool).
        let tiles: Vec<Option<(usize, Vec<u8>)>> = omt_render::workers::install(threads, || {
            use rayon::prelude::*;
            (tx0..=tx1)
                .into_par_iter()
                .map(|tx| {
                    let key = TileKey::new(job.level, tx as i32, tz as i32);
                    let t = omt_render::build_tile(src, key, None, Palette::embedded(), &opts, &cancelled)?;
                    done.fetch_add(1, Ordering::Relaxed);
                    if t.px == 0 {
                        return Some((0, Vec::new()));
                    }
                    let view = omt_render::style::TileView { px: t.px as usize, bpp: span_blocks as f32 / t.px as f32, rgba: &t.rgba, heights: &t.heights, surface: &t.surface, biomes: &t.biomes, names: &t.names };
                    Some((t.px as usize, omt_render::style::stylize(&view, &style)))
                })
                .collect()
        });
        // Output rows covered by this tile row.
        let tile_z0 = tz * span_blocks;
        let first = ((job.z0.max(tile_z0) - job.z0) / bpp) as usize;
        let last = ((job.z1.min(tile_z0 + span_blocks) - job.z0 + bpp - 1) / bpp) as usize;
        let bg = job.background.map(|c| [c[0], c[1], c[2], 255]);
        let mut line = vec![0u8; row_px * 4];
        let mut scaled = vec![0u8; w as usize * 4];
        for oy in first..last {
            let bz = job.z0 + oy as i64 * bpp;
            let ty = ((bz - tile_z0) / bpp) as usize;
            for (ox, px) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let bx = job.x0 + ox as i64 * bpp;
                let tx = bx.div_euclid(span_blocks);
                let lx = ((bx - tx * span_blocks) / bpp) as usize;
                let src_px = tiles.get((tx - tx0) as usize).and_then(|t| t.as_ref()).and_then(|(tpx, rgba)| rgba.get((ty * tpx + lx) * 4..(ty * tpx + lx) * 4 + 4));
                *px = match (src_px, bg) {
                    (Some(p), _) if p[3] != 0 => [p[0], p[1], p[2], p[3]],
                    (_, Some(b)) => b,
                    _ => [0; 4],
                };
            }
            if job.scale == 1 {
                std::io::Write::write_all(&mut wr, &line).map_err(|e| e.to_string())?;
            } else {
                for (i, px) in line.as_chunks::<4>().0.iter().enumerate() {
                    for k in 0..job.scale as usize {
                        let o = (i * job.scale as usize + k) * 4;
                        scaled[o..o + 4].copy_from_slice(px);
                    }
                }
                for _ in 0..job.scale {
                    std::io::Write::write_all(&mut wr, &scaled).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    wr.finish().map_err(|e| e.to_string())?;
    Ok(format!("exported {w}×{h} px to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_and_job_sizes() {
        assert_eq!(Zoom(-2).level_scale(), (2, 1));
        assert_eq!(Zoom(2).level_scale(), (0, 4));
        let job = AreaJob { x0: -100, z0: 0, x1: 100, z1: 50, level: 1, scale: 1, background: None };
        assert_eq!(job.px_size(), (100, 25));
        let up = AreaJob { level: 0, scale: 2, ..job };
        assert_eq!(up.px_size(), (400, 100));
    }

    /// Fixture-gated: `OMT_EXPORT_FIXTURE=<java world>` exports a 1:2 image
    /// of the world's core and checks the PNG.
    #[test]
    #[ignore = "needs OMT_EXPORT_FIXTURE=<java world folder>"]
    fn export_area_writes_png() {
        let Ok(root) = std::env::var("OMT_EXPORT_FIXTURE") else { return };
        let w = std::sync::Arc::new(omt_java::JavaWorld::open(std::path::Path::new(&root)).unwrap());
        let src = MapSource::java(&w, 0).unwrap();
        let (a, b, c, d) = src.chunks().core_bounds(0.02).unwrap();
        let job = AreaJob { x0: a as i64 * 16, z0: b as i64 * 16, x1: (c as i64 + 1) * 16, z1: (d as i64 + 1) * 16, level: 1, scale: 1, background: Some([10, 20, 30]) };
        let out = std::env::temp_dir().join(format!("omt-export-{}.png", std::process::id()));
        let look = Look { opts: RenderOptions { relief: false, ..Default::default() }, style: StyleParams::default(), threads: 2 };
        let msg = export_area(&src, look, job, &out, &AtomicBool::new(false), &AtomicU64::new(0)).unwrap();
        let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&out).unwrap()));
        let r = dec.read_info().unwrap();
        let (w, h) = job.px_size();
        assert_eq!((r.info().width as u64, r.info().height as u64), (w, h), "{msg}");
        let _ = std::fs::remove_file(out);
    }
}
