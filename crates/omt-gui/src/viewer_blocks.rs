//! Builds the block grid for the 3D "Blocks" detail: decoded chunks around
//! the camera → `omt_render::blocks3d::BlockGrid`, with each block kind's
//! top/side textures from the active resource pack (inventory-colored
//! variants for grass/leaves, tinted water) or its map color.

use std::collections::HashMap;

use omt_art::Image;
use omt_render::blocks3d::{BlockGrid, Kind};
use omt_texpack::{Face, Pack};

use crate::session::Store;

/// Horizontal radius (blocks) loaded around the camera (CPU ray-casting).
pub const RADIUS: i32 = 96;

fn invisible(name: &str) -> bool {
    matches!(
        name.trim_start_matches("minecraft:"),
        "air" | "cave_air" | "void_air" | "light_block" | "light" | "structure_void" | "barrier" | "moving_block" | "border_block"
    ) || name.contains("light_block")
}

fn tex16(img: &Image) -> Vec<[u8; 4]> {
    (0..256)
        .map(|i| {
            let (x, y) = (i % 16, i / 16);
            img.get((x * img.w as usize / 16) as i32, (y * img.h as usize / 16) as i32)
        })
        .collect()
}

fn tinted(tex: Vec<[u8; 4]>, t: [u8; 3]) -> Vec<[u8; 4]> {
    tex.into_iter().map(|p| [(p[0] as u32 * t[0] as u32 / 255) as u8, (p[1] as u32 * t[1] as u32 / 255) as u8, (p[2] as u32 * t[2] as u32 / 255) as u8, p[3]]).collect()
}

fn is_gray(tex: &[[u8; 4]]) -> bool {
    !tex.iter().filter(|p| p[3] > 0).any(|p| p[0].abs_diff(p[1]) > 12 || p[1].abs_diff(p[2]) > 12)
}

/// The look of one block (texture pack when available).
pub fn kind_of(name: &str, pack: Option<&Pack>) -> Kind {
    let n = name.trim_start_matches("minecraft:");
    let water = n == "water" || n == "flowing_water" || n.contains("bubble_column");
    let tint = if water {
        Some([0x3f, 0x76, 0xe4])
    } else if n.contains("leaves") || n == "vine" {
        Some([0x59, 0xae, 0x30])
    } else if n == "grass_block" || n.contains("grass") || n.contains("fern") {
        Some([0x79, 0xc0, 0x5a])
    } else {
        None
    };
    if let Some(p) = pack {
        let top = p.face(name, Face::Up, true).or_else(|| p.face(name, Face::Up, false));
        let side = p.face(name, Face::Side, false).or_else(|| top.clone());
        if let (Some(t), Some(s)) = (top, side) {
            let (mut top, mut side) = (tex16(&t), tex16(&s));
            if let Some(c) = tint {
                if is_gray(&top) {
                    top = tinted(top, c);
                }
                if is_gray(&side) && n != "grass_block" {
                    side = tinted(side, c);
                }
            }
            if n == "grass_block" {
                // Side is an overlay over dirt in Bedrock packs.
                if let Some(d) = p.face("dirt", Face::Up, false) {
                    let dirt = tex16(&d);
                    let fringe = if is_gray(&side) { tinted(side.clone(), [0x79, 0xc0, 0x5a]) } else { side.clone() };
                    side = dirt.iter().zip(&fringe).map(|(d, f)| if f[3] >= 128 { *f } else { *d }).collect();
                }
            }
            return Kind { top, side, water };
        }
    }
    let c = omt_render::Palette::embedded().color_of(name);
    let c = match tint {
        Some(t) if water => [t[0], t[1], t[2], 255],
        Some(t) => [(c[0] as u32 * t[0] as u32 / 255) as u8, (c[1] as u32 * t[1] as u32 / 255) as u8, (c[2] as u32 * t[2] as u32 / 255) as u8, 255],
        None => c,
    };
    Kind { water, ..Kind::flat([c[0], c[1], c[2], 255]) }
}

/// Read chunks around (`cx`, `cz`) and build the grid (blocking).
pub fn build(store: &Store, dim: u8, cx: f64, cz: f64, eye_y: f32) -> Result<BlockGrid, String> {
    build_r(store, dim, cx, cz, eye_y, RADIUS)
}

/// [`build`] with an explicit horizontal radius.
pub fn build_r(store: &Store, dim: u8, cx: f64, cz: f64, eye_y: f32, radius: i32) -> Result<BlockGrid, String> {
    let (bx, bz) = (cx.floor() as i32, cz.floor() as i32);
    let (x0, z0) = (bx - radius, bz - radius);
    let (lo, hi) = omt_edit::y_range(dim);
    // Vertical window: well below the eye (terrain, caves under foot) and a
    // little above it.
    let y0 = ((eye_y as i32) - 140).max(lo);
    let y1 = ((eye_y as i32) + 60).min(hi);
    let size = [(2 * radius) as usize, (y1 - y0 + 1).max(1) as usize, (2 * radius) as usize];
    let mut grid = BlockGrid::empty([x0, y0, z0], size);
    let pack = crate::textures::current();
    let mut ids: HashMap<String, u16> = HashMap::new();
    let world = store.world();
    let chunks: Vec<(i32, i32)> = ((z0.div_euclid(16))..=((z0 + 2 * radius - 1).div_euclid(16))).flat_map(|z| ((x0.div_euclid(16))..=((x0 + 2 * radius - 1).div_euclid(16))).map(move |x| (x, z))).collect();
    use rayon::prelude::*;
    let decoded: Vec<_> = chunks.par_iter().filter_map(|&(x, z)| world.read_chunk(dim, x, z).ok().flatten()).collect();
    for ch in decoded {
        for (sy, slice) in &ch.slices {
            let base_y = *sy as i32 * 16;
            if base_y + 15 < y0 || base_y > y1 {
                continue;
            }
            // Palette → grid kind ids once per section.
            let map: Vec<u16> = slice
                .palette
                .iter()
                .map(|s| {
                    if invisible(&s.name) {
                        return 0;
                    }
                    *ids.entry(s.name.clone()).or_insert_with(|| {
                        grid.kinds.push(kind_of(&s.name, pack.as_deref()));
                        (grid.kinds.len() - 1) as u16
                    })
                })
                .collect();
            for (i, &p) in slice.indices.iter().enumerate() {
                let id = map.get(p as usize).copied().unwrap_or(0);
                if id == 0 {
                    continue;
                }
                // YZX in-section order.
                let (ly, lz, lx) = ((i / 256) as i32, ((i / 16) % 16) as i32, (i % 16) as i32);
                grid.set(ch.x * 16 + lx, base_y + ly, ch.z * 16 + lz, id);
            }
        }
    }
    Ok(grid)
}
