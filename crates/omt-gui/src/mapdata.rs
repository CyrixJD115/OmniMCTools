//! Map data for the canvas: region-sized image tiles (32×32 chunks →
//! 512×512 px) plus per-chunk surface info for click-identify. Built in a
//! background thread from the session's store.

use std::collections::{HashMap, HashSet};

use iced::widget::image::Handle;
use omt_render::{Palette, RenderOptions};

use crate::session::Store;

pub const TILE_PX: u32 = 512;

/// Surface info of one chunk: block-name ids (into `MapData::names`) and
/// heights per column (`i16::MIN` = empty).
#[derive(Debug, Clone)]
pub struct Surface {
    pub names: Box<[u16; 256]>,
    pub heights: Box<[i16; 256]>,
}

/// Raw output of the background build (no GPU handles yet).
#[derive(Debug, Default)]
pub struct MapBuild {
    pub dim: u8,
    pub tiles: Vec<((i32, i32), Vec<u8>)>,
    pub chunks: HashSet<(i32, i32)>,
    pub surface: HashMap<(i32, i32), Surface>,
    pub names: Vec<String>,
    pub errors: usize,
    pub millis: u128,
}

/// Map ready for the canvas.
#[derive(Debug, Default, Clone)]
pub struct MapData {
    pub tiles: Vec<(i32, i32, Handle)>,
    pub chunks: HashSet<(i32, i32)>,
    pub surface: std::sync::Arc<HashMap<(i32, i32), Surface>>,
    pub names: std::sync::Arc<Vec<String>>,
    pub errors: usize,
    pub millis: u128,
}

impl MapData {
    pub fn from_build(b: MapBuild) -> Self {
        Self {
            tiles: b
                .tiles
                .into_iter()
                .map(|((rx, rz), rgba)| (rx, rz, Handle::from_rgba(TILE_PX, TILE_PX, rgba)))
                .collect(),
            chunks: b.chunks,
            surface: std::sync::Arc::new(b.surface),
            names: std::sync::Arc::new(b.names),
            errors: b.errors,
            millis: b.millis,
        }
    }

    /// Surface block + height at a block position, if rendered.
    pub fn identify(&self, bx: i32, bz: i32) -> Option<(&str, i32)> {
        let (cx, cz) = (bx.div_euclid(16), bz.div_euclid(16));
        let s = self.surface.get(&(cx, cz))?;
        let i = (bz.rem_euclid(16) * 16 + bx.rem_euclid(16)) as usize;
        let h = s.heights[i];
        if h == i16::MIN {
            return None;
        }
        Some((self.names.get(s.names[i] as usize)?.as_str(), h as i32))
    }

    /// Bounding box of present chunks (min_cx, min_cz, max_cx, max_cz).
    pub fn chunk_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let mut it = self.chunks.iter();
        let first = *it.next()?;
        Some(it.fold((first.0, first.1, first.0, first.1), |b, &(x, z)| (b.0.min(x), b.1.min(z), b.2.max(x), b.3.max(z))))
    }
}

/// Render every chunk of a dimension (blocking; call via `session::blocking`).
pub fn build(store: Store, dim: u8, opts: RenderOptions) -> Result<MapBuild, String> {
    let started = std::time::Instant::now();
    let coords: Vec<(i32, i32)> = store.world().iter_chunks(dim).map_err(|e| e.to_string())?.collect();
    let (renders, errors) = omt_render::render_chunks(store.world(), dim, &coords, Palette::embedded(), &opts);
    let mut tiles: HashMap<(i32, i32), Vec<u8>> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    let mut name_ids: HashMap<String, u16> = HashMap::new();
    let mut surface = HashMap::new();
    let mut chunks = HashSet::new();
    for ((cx, cz), r) in renders {
        chunks.insert((cx, cz));
        let (rx, rz) = (cx.div_euclid(32), cz.div_euclid(32));
        let tile = tiles.entry((rx, rz)).or_insert_with(|| vec![0u8; (TILE_PX * TILE_PX * 4) as usize]);
        let (ox, oz) = (cx.rem_euclid(32) as usize * 16, cz.rem_euclid(32) as usize * 16);
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
    chunks.extend(coords.iter().copied());
    Ok(MapBuild {
        dim,
        tiles: tiles.into_iter().collect(),
        chunks,
        surface,
        names,
        errors,
        millis: started.elapsed().as_millis(),
    })
}
