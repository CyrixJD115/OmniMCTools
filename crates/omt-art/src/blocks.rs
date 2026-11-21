//! Procedural 16×16 block textures (original, noise-based — not Mojang's
//! art) and an isometric block renderer for tool tiles and thumbnails.

use crate::image::{Image, Rgba, Rng, hash2, hex, mix, shade};

/// Materials the generators know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Material {
    GrassTop,
    GrassSide,
    Dirt,
    Stone,
    Deepslate,
    Bedrock,
    Sand,
    Water,
    LogSide,
    LogTop,
    Leaves,
    Planks,
    Netherrack,
    EndStone,
    Obsidian,
    Snow,
    DiamondOre,
    GoldBlock,
    Redstone,
    Amethyst,
    Glass,
    Copper,
}

/// A 4-shade ramp, darkest first.
fn ramp(m: Material) -> [Rgba; 4] {
    use Material::*;
    match m {
        GrassTop => [hex(0x3f7a2a), hex(0x4f9433), hex(0x5ea83b), hex(0x6fbe45)],
        GrassSide | Dirt => [hex(0x5a3d26), hex(0x6e4b2f), hex(0x80583a), hex(0x936847)],
        Stone | DiamondOre => [hex(0x6b6b6e), hex(0x7d7d80), hex(0x8e8e91), hex(0x9f9fa2)],
        Deepslate => [hex(0x2e2e34), hex(0x3a3a41), hex(0x46464e), hex(0x53535c)],
        Bedrock => [hex(0x1e1e20), hex(0x3c3c3f), hex(0x5d5d60), hex(0x808083)],
        Sand => [hex(0xcbb57c), hex(0xd8c48b), hex(0xe2cf98), hex(0xeddca8)],
        Water => [hex(0x2a4fb8), hex(0x3260cc), hex(0x3a6fdc), hex(0x4880e8)],
        LogSide | LogTop => [hex(0x4a3420), hex(0x5c4128), hex(0x6d4e31), hex(0x7f5d3b)],
        Leaves => [hex(0x24581c), hex(0x2f6c24), hex(0x3b822d), hex(0x4a9838)],
        Planks => [hex(0x8c6a3f), hex(0x9e7a4a), hex(0xb08a55), hex(0xc19a62)],
        Netherrack => [hex(0x5e1a1a), hex(0x742424), hex(0x8a2e2e), hex(0x9e3a36)],
        EndStone => [hex(0xc9c58e), hex(0xd8d49c), hex(0xe3dfa9), hex(0xeeeab8)],
        Obsidian => [hex(0x0e0a18), hex(0x1a1228), hex(0x2a1d40), hex(0x3c2a5a)],
        Snow => [hex(0xd9e2ea), hex(0xe6edf3), hex(0xf0f5f9), hex(0xfbfdff)],
        GoldBlock => [hex(0xc9951c), hex(0xe0ad2a), hex(0xf2c94c), hex(0xfde07a)],
        Redstone => [hex(0x6e0b0b), hex(0x9c1414), hex(0xc81e1e), hex(0xf03a2a)],
        Amethyst => [hex(0x4b2d7a), hex(0x6340a0), hex(0x7d58c0), hex(0xa07be0)],
        Glass => [hex(0xa8d8e8), hex(0xc0e6f2), hex(0xd8f2fa), hex(0xf2fcff)],
        Copper => [hex(0x9a4f32), hex(0xb8623f), hex(0xd27a50), hex(0xe89a6c)],
    }
}

/// Clustered noise index 0..4 (soft 4-px patches + edge jitter).
fn blob(seed: u64, x: i32, y: i32) -> usize {
    // Patches, not salt: a coarse value noise (bilinear over 4-px cells,
    // wrapping so the texture tiles) dominates; fine noise only breaks the
    // patch edges.
    let cell = |cx: i32, cy: i32| hash2(seed, cx.rem_euclid(4), cy.rem_euclid(4));
    let (fx, fy) = ((x as f32 + 0.5) / 4.0, (y as f32 + 0.5) / 4.0);
    let (cx, cy) = (fx.floor() as i32, fy.floor() as i32);
    let (tx, ty) = (fx - cx as f32, fy - cy as f32);
    let top = cell(cx, cy) * (1.0 - tx) + cell(cx + 1, cy) * tx;
    let bot = cell(cx, cy + 1) * (1.0 - tx) + cell(cx + 1, cy + 1) * tx;
    let coarse = top * (1.0 - ty) + bot * ty;
    let fine = hash2(seed ^ 0xabcdef, x, y);
    ((coarse * 0.75 + fine * 0.25) * 4.0 - 0.3).clamp(0.0, 3.999) as usize
}

/// A 16×16 texture for a material.
pub fn texture(m: Material) -> Image {
    use Material::*;
    let seed = m as u64 * 1_000_003 + 17;
    let r = ramp(m);
    let mut img = Image::new(16, 16);
    let mut rng = Rng::new(seed);
    for y in 0..16 {
        for x in 0..16 {
            let mut c = r[blob(seed, x, y)];
            match m {
                GrassSide => {
                    // Ragged grass lip over dirt.
                    let lip = 3 + (hash2(seed, x, 99) * 2.6) as i32;
                    if y < lip {
                        c = ramp(GrassTop)[blob(seed + 1, x, y).min(3)];
                    } else if y == lip {
                        c = shade(c, 0.85);
                    }
                }
                LogSide => {
                    // Vertical bark ridges.
                    let ridge = (x * 7 + (hash2(seed, x, 0) * 3.0) as i32) % 4;
                    c = r[(ridge as usize + (y % 5 == 0) as usize).min(3)];
                }
                LogTop => {
                    let d = (((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt()) as i32;
                    c = if d >= 7 { r[0] } else { ramp(Planks)[((d as usize % 2) + 1 + (d < 2) as usize).min(3)] };
                }
                Planks => {
                    let seam = y % 4 == 3;
                    let joint = (x + if (y / 4) % 2 == 0 { 0 } else { 8 }) % 16 == 0;
                    c = if seam || joint { r[0] } else { r[1 + (hash2(seed, x / 3, y / 4) * 2.99) as usize] };
                }
                Water => {
                    let wave = ((x + y * 2) % 8 < 2) as usize;
                    c = r[(1 + wave + (hash2(seed, x, y) > 0.8) as usize).min(3)];
                }
                Leaves => {
                    if hash2(seed, x, y) > 0.86 {
                        c = shade(r[0], 0.6);
                    }
                }
                DiamondOre => {
                    if ore(seed, x, y) {
                        c = [hex(0x2a9fb0), hex(0x45d0dd), hex(0x8ff0f0), hex(0xd0ffff)][blob(seed + 5, x, y)];
                    }
                }
                GoldBlock | Copper | Redstone => {
                    // Metal block: bevelled rim, brushed body, a diagonal
                    // glint across the upper left.
                    c = if x == 0 || y == 0 {
                        r[3]
                    } else if x == 15 || y == 15 {
                        r[0]
                    } else if x == 1 || y == 1 {
                        mix(r[2], r[3], 0.5)
                    } else if x == 14 || y == 14 {
                        mix(r[0], r[1], 0.5)
                    } else if (x + y == 9 || x + y == 10) && x < 8 {
                        r[3]
                    } else {
                        r[1 + ((hash2(seed, x / 4, y) > 0.55) as usize)]
                    };
                }
                Amethyst => {
                    // Crystal facets: diagonal bands with bright edges.
                    let band = (x + 2 * y + (hash2(seed, (x + 2 * y) / 5, 0) * 3.0) as i32).rem_euclid(7);
                    c = match band {
                        0 => r[3],
                        1 | 2 => r[2],
                        3 | 4 => r[1],
                        _ => r[0],
                    };
                }
                Deepslate => {
                    // Horizontal strata with offset seams.
                    let row = y / 3;
                    let seam = y % 3 == 2 && (x + row * 5).rem_euclid(9) != 0;
                    c = if seam { shade(r[0], 0.85) } else { r[(blob(seed, x, row * 3) + (row % 2) as usize).min(3)] };
                }
                Netherrack => {
                    // Dark veins between lumpy patches.
                    let v = hash2(seed, (x + y / 3) / 3, y / 2);
                    c = if v > 0.82 { shade(r[0], 0.7) } else { r[blob(seed, x, y)] };
                }
                EndStone => {
                    if hash2(seed ^ 0x55, x, y) > 0.9 {
                        c = shade(r[0], 0.85);
                    }
                }
                Glass => {
                    let frame = x == 0 || y == 0 || x == 15 || y == 15;
                    let glint = (x - y == 3 || x - y == 4) && (3..9).contains(&x);
                    c = if frame { r[1] } else if glint { r[3] } else { [r[2][0], r[2][1], r[2][2], 70] };
                }
                Snow if rng.chance(0.04) => c = hex(0xffffff),
                _ => {}
            }
            img.set(x, y, c);
        }
    }
    img
}

fn ore(seed: u64, x: i32, y: i32) -> bool {
    // Three small veins at fixed noise-chosen centers.
    (0..3).any(|i| {
        let cx = 3 + (hash2(seed, i, 1) * 10.0) as i32;
        let cy = 3 + (hash2(seed, i, 2) * 10.0) as i32;
        (x - cx).abs() + (y - cy).abs() <= 1 + (hash2(seed, x, y) > 0.6) as i32
    })
}

/// Faces of an isometric block.
#[derive(Debug, Clone)]
pub struct BlockFaces {
    pub top: Image,
    pub left: Image,
    pub right: Image,
}

impl BlockFaces {
    /// Same texture on every face.
    pub fn uniform(m: Material) -> Self {
        let t = texture(m);
        Self { top: t.clone(), left: t.clone(), right: t }
    }

    /// Grass block (grass top, grass-side sides).
    pub fn grass() -> Self {
        let side = texture(Material::GrassSide);
        Self { top: texture(Material::GrassTop), left: side.clone(), right: side }
    }

    /// Log (rings on top, bark on the sides).
    pub fn log() -> Self {
        let side = texture(Material::LogSide);
        Self { top: texture(Material::LogTop), left: side.clone(), right: side }
    }

    pub fn of(m: Material) -> Self {
        match m {
            Material::GrassTop | Material::GrassSide => Self::grass(),
            Material::LogSide | Material::LogTop => Self::log(),
            other => Self::uniform(other),
        }
    }
}

/// Render a block in 2:1 isometric projection. Output is `2s × 2s`
/// pixels for 16-pixel faces scaled by `s / 16` (use `s = 16` for a 32×32
/// icon). Left face is shaded darker, right face medium (Minecraft-style
/// inventory lighting).
pub fn iso_block(faces: &BlockFaces, s: u32) -> Image {
    let size = 2 * s;
    let mut img = Image::new(size, size);
    let sf = s as f32;
    // Inverse-map every output pixel to a face texel.
    for py in 0..size as i32 {
        for px in 0..size as i32 {
            let x = px as f32 + 0.5 - sf; // centered
            let y = py as f32 + 0.5;
            // Top rhombus: center (0, s/2), half-width s, half-height s/2.
            let u = (y / (sf / 2.0) + x / sf) / 2.0; // 0..1 along +x edge
            let v = (y / (sf / 2.0) - x / sf) / 2.0;
            if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
                let c = faces.top.get((u * faces.top.w as f32) as i32, (v * faces.top.h as f32) as i32);
                img.set(px, py, c);
                continue;
            }
            // Side faces below the top rhombus.
            if x < 0.0 {
                // Left face: spans x in [-s, 0], top edge y = s/2 + (x+s)/2.
                let fx = (x + sf) / sf;
                let top_edge = sf / 2.0 + (x + sf) / 2.0;
                let fy = (y - top_edge) / sf;
                if (0.0..1.0).contains(&fx) && (0.0..1.0).contains(&fy) {
                    let c = faces.left.get((fx * faces.left.w as f32) as i32, (fy * faces.left.h as f32) as i32);
                    // Side faces darken a little toward the ground.
                    img.set(px, py, shade(c, 0.66 - 0.08 * fy));
                }
            } else {
                let fx = x / sf;
                let top_edge = sf - x / 2.0;
                let fy = (y - top_edge) / sf;
                if (0.0..1.0).contains(&fx) && (0.0..1.0).contains(&fy) {
                    let c = faces.right.get((fx * faces.right.w as f32) as i32, (fy * faces.right.h as f32) as i32);
                    img.set(px, py, shade(c, 0.84 - 0.08 * fy));
                }
            }
        }
    }
    // Soft rim light on the top edges for a modern look.
    for px in 0..size as i32 {
        for py in 0..size as i32 {
            let c = img.get(px, py);
            if c[3] > 0 && img.get(px, py - 1)[3] == 0 {
                img.set(px, py, mix(c, hex(0xffffff), 0.25));
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textures_are_opaque_and_deterministic() {
        for m in [Material::GrassTop, Material::Stone, Material::Planks, Material::DiamondOre] {
            let a = texture(m);
            assert_eq!(a, texture(m));
            assert_eq!(a.coverage(), 1.0);
        }
        // Glass is partly see-through.
        assert!(texture(Material::Glass).rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 255));
    }

    #[test]
    fn iso_block_shape() {
        let img = iso_block(&BlockFaces::grass(), 16);
        assert_eq!((img.w, img.h), (32, 32));
        // Center filled, corners empty (hexagon silhouette).
        assert!(img.get(16, 16)[3] > 0);
        assert_eq!(img.get(0, 0)[3], 0);
        assert_eq!(img.get(31, 0)[3], 0);
        let cov = img.coverage();
        assert!((0.6..0.8).contains(&cov), "coverage {cov}");
        // Left face darker than right face.
        let l = img.get(8, 22);
        let r = img.get(24, 22);
        assert!(l[0] as u32 + l[1] as u32 + l[2] as u32 <= r[0] as u32 + r[1] as u32 + r[2] as u32 + 60);
    }
}
