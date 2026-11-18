//! Map Viewer "block textures" layer: when zoomed in, the map is redrawn
//! from the real top textures of the active resource pack, in cells of
//! 32×32 blocks built on background threads. Each texture is modulated by
//! the map's styled color for that block, so biome tint, sun lighting,
//! Night glow and water depth all carry over to the textured view.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use iced::widget::image::Handle;
use omt_art::Image;
use omt_texpack::Face;

use crate::mapdata::MapData;

/// Blocks per cell edge.
pub const CELL: i32 = 32;
/// Texture pixels per block in a built cell.
const PX: usize = 16;
/// Screen pixels per block at which textures switch on.
pub const MIN_ZOOM: f32 = 6.0;
/// Cells kept in memory (each 512×512 RGBA = 1 MiB).
const KEEP: usize = 96;

/// One cell's inputs, gathered on the UI thread.
pub struct CellJob {
    pub key: (i32, i32),
    pub stamp: u64,
    /// Per block (row-major): texture (if any) and the styled map color.
    blocks: Vec<(Option<Arc<Image>>, [u8; 4])>,
}

/// Mean color of a texture's opaque pixels.
fn mean(img: &Image) -> [f32; 3] {
    let (mut acc, mut n) = ([0f32; 3], 0f32);
    for p in img.rgba.as_chunks::<4>().0 {
        if p[3] > 8 {
            for k in 0..3 {
                acc[k] += p[k] as f32;
            }
            n += 1.0;
        }
    }
    if n == 0.0 { [1.0; 3] } else { [acc[0] / n, acc[1] / n, acc[2] / n] }
}

impl CellJob {
    /// Build the cell image (worker thread).
    pub fn build(self) -> ((i32, i32), u64, Option<Handle>) {
        let edge = CELL as usize * PX;
        let mut out = vec![0u8; edge * edge * 4];
        let mut means: HashMap<*const Image, [f32; 3]> = HashMap::new();
        let mut any = false;
        for (i, (tex, c)) in self.blocks.iter().enumerate() {
            if c[3] == 0 {
                continue;
            }
            any = true;
            let (bx, bz) = (i % CELL as usize, i / CELL as usize);
            match tex {
                Some(t) if t.w > 0 && t.h > 0 => {
                    let m = *means.entry(Arc::as_ptr(t)).or_insert_with(|| mean(t));
                    let gain = [c[0] as f32 / m[0].max(1.0), c[1] as f32 / m[1].max(1.0), c[2] as f32 / m[2].max(1.0)];
                    for py in 0..PX {
                        for px in 0..PX {
                            let s = t.get((px * t.w as usize / PX) as i32, (py * t.h as usize / PX) as i32);
                            let o = ((bz * PX + py) * edge + bx * PX + px) * 4;
                            if s[3] < 128 {
                                // Cut-out pixels (flowers, grass) show the map color.
                                out[o..o + 3].copy_from_slice(&c[..3]);
                            } else {
                                for k in 0..3 {
                                    out[o + k] = (s[k] as f32 * gain[k]).round().clamp(0.0, 255.0) as u8;
                                }
                            }
                            out[o + 3] = 255;
                        }
                    }
                }
                _ => {
                    for py in 0..PX {
                        let o = ((bz * PX + py) * edge + bx * PX) * 4;
                        for px in 0..PX {
                            out[o + px * 4..o + px * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                        }
                    }
                }
            }
        }
        (self.key, self.stamp, any.then(|| Handle::from_rgba(edge as u32, edge as u32, out)))
    }
}

/// Built cells + bookkeeping.
#[derive(Default)]
pub struct TexLayer {
    pub cells: HashMap<(i32, i32), Handle>,
    order: Vec<(i32, i32)>,
    pending: HashSet<(i32, i32)>,
    /// Inputs version (style, pack, render options, dimension).
    pub stamp: u64,
}

impl TexLayer {
    /// Drop everything (inputs changed).
    pub fn reset(&mut self, stamp: u64) {
        if stamp != self.stamp {
            self.cells.clear();
            self.order.clear();
            self.pending.clear();
            self.stamp = stamp;
        }
    }

    pub fn insert(&mut self, key: (i32, i32), stamp: u64, h: Option<Handle>) {
        self.pending.remove(&key);
        if stamp != self.stamp {
            return;
        }
        if let Some(h) = h {
            self.cells.insert(key, h);
            self.order.retain(|k| *k != key);
            self.order.push(key);
            while self.order.len() > KEEP {
                let old = self.order.remove(0);
                self.cells.remove(&old);
            }
        }
    }

    /// Jobs for visible cells not built yet (at most `max`; only cells
    /// whose full-detail tiles are loaded).
    pub fn plan(&mut self, map: &MapData, visible: (f64, f64, f64, f64), max: usize) -> Vec<CellJob> {
        let Some(pack) = crate::textures::current() else { return Vec::new() };
        let (x0, z0, x1, z1) = visible;
        let c = |v: f64| (v.floor() as i64).div_euclid(CELL as i64) as i32;
        let (cx0, cz0, cx1, cz1) = (c(x0), c(z0), c(x1), c(z1));
        let mut jobs = Vec::new();
        let mut names: HashMap<String, Option<Arc<Image>>> = HashMap::new();
        'cells: for cz in cz0..=cz1 {
            for cx in cx0..=cx1 {
                if jobs.len() >= max {
                    break 'cells;
                }
                let key = (cx, cz);
                if self.cells.contains_key(&key) || self.pending.contains(&key) {
                    continue;
                }
                let mut blocks = Vec::with_capacity((CELL * CELL) as usize);
                let mut ready = true;
                for bz in 0..CELL {
                    for bx in 0..CELL {
                        let (wx, wz) = (cx * CELL + bx, cz * CELL + bz);
                        match map.block_sample(wx, wz) {
                            Some((name, col)) => {
                                let tex = names.entry(name.to_string()).or_insert_with(|| pack.face(name, Face::Up, false)).clone();
                                blocks.push((tex, col));
                            }
                            None => {
                                if map.chunk_present(wx.div_euclid(16), wz.div_euclid(16)) && !map.full_tile_loaded(wx, wz) {
                                    // Its full-detail tile is not loaded yet.
                                    ready = false;
                                }
                                blocks.push((None, [0; 4]));
                            }
                        }
                    }
                }
                if ready {
                    self.pending.insert(key);
                    jobs.push(CellJob { key, stamp: self.stamp, blocks });
                }
            }
        }
        jobs
    }
}
