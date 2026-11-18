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
