//! Whole-area maps: render many chunks (parallel), shade across chunk
//! borders, assemble into one RGBA image, and export PNG.

use std::collections::HashMap;
use std::path::Path;

use omt_world::{WorldResult, WorldStore};
use rayon::prelude::*;

use crate::palette::Palette;
use crate::render::{ChunkRender, RenderOptions, render_chunk, shade};

/// Render the listed chunks of a dimension. Chunks that fail to decode are
/// skipped (counted in the returned error total) — a map must never abort
/// on one corrupt column.
pub fn render_chunks(
    store: &(dyn WorldStore + Sync),
    dim: u8,
    coords: &[(i32, i32)],
    palette: &Palette,
    opts: &RenderOptions,
) -> (HashMap<(i32, i32), ChunkRender>, usize) {
    let results: Vec<(Option<ChunkRender>, bool)> = coords
        .par_iter()
        .map(|&(x, z)| match store.read_chunk(dim, x, z) {
            Ok(Some(chunk)) => (Some(render_chunk(&chunk, palette, opts)), false),
            Ok(None) => (None, false),
            Err(_) => (None, true),
        })
        .collect();
    let errors = results.iter().filter(|r| r.1).count();
    let mut map: HashMap<(i32, i32), ChunkRender> =
        results.into_iter().filter_map(|r| r.0).map(|r| ((r.x, r.z), r)).collect();
    shade_all(&mut map);
    (map, errors)
}

/// Cross-chunk height shading (northern neighbor). Shared by the store path
/// and the Bedrock snapshot fast path.
pub fn shade_all(map: &mut HashMap<(i32, i32), ChunkRender>) {
    let norths: HashMap<(i32, i32), [i32; 256]> = map.iter().map(|(k, r)| (*k, r.heights)).collect();
    for ((x, z), r) in map.iter_mut() {
        shade(r, norths.get(&(*x, *z - 1)));
    }
}

/// A stitched map image; pixel (0,0) is block (`min_cx*16`, `min_cz*16`).
#[derive(Debug, Clone)]
pub struct MapImage {
    pub min_cx: i32,
    pub min_cz: i32,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl MapImage {
    /// Stitch rendered chunks (absent chunks stay transparent).
    pub fn stitch(chunks: &HashMap<(i32, i32), ChunkRender>) -> Option<Self> {
        let (min_cx, max_cx) = chunks.keys().map(|k| k.0).fold((i32::MAX, i32::MIN), |a, x| (a.0.min(x), a.1.max(x)));
        let (min_cz, max_cz) = chunks.keys().map(|k| k.1).fold((i32::MAX, i32::MIN), |a, z| (a.0.min(z), a.1.max(z)));
        if chunks.is_empty() {
            return None;
        }
        let width = ((max_cx - min_cx + 1) * 16) as usize;
        let height = ((max_cz - min_cz + 1) * 16) as usize;
        let mut rgba = vec![0u8; width * height * 4];
        for ((cx, cz), r) in chunks {
            let ox = ((cx - min_cx) * 16) as usize;
            let oz = ((cz - min_cz) * 16) as usize;
            for lz in 0..16 {
                let dst = ((oz + lz) * width + ox) * 4;
                rgba[dst..dst + 64].copy_from_slice(&r.rgba[lz * 64..lz * 64 + 64]);
            }
        }
        Some(Self { min_cx, min_cz, width, height, rgba })
    }

    pub fn write_png(&self, path: &Path) -> std::io::Result<()> {
        let file = std::fs::File::create(path)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), self.width as u32, self.height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(std::io::Error::other)?;
        w.write_image_data(&self.rgba).map_err(std::io::Error::other)?;
        w.finish().map_err(std::io::Error::other)
    }
}

/// Convenience: render every present chunk of a dimension (optionally
/// limited to a chunk-space box) into one image.
pub fn render_world(
    store: &(dyn WorldStore + Sync),
    dim: u8,
    bounds: Option<(i32, i32, i32, i32)>,
    opts: &RenderOptions,
) -> WorldResult<(Option<MapImage>, usize, usize)> {
    let mut coords: Vec<(i32, i32)> = store.iter_chunks(dim)?.collect();
    if let Some((x0, z0, x1, z1)) = bounds {
        coords.retain(|&(x, z)| x >= x0 && x <= x1 && z >= z0 && z <= z1);
    }
    let (chunks, errors) = render_chunks(store, dim, &coords, Palette::embedded(), opts);
    Ok((MapImage::stitch(&chunks), chunks.len(), errors))
}
