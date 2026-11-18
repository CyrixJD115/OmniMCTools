//! Procedural pixel-art panoramas: banded/dithered skies, a sun or moon,
//! blocky clouds, aerial-perspective ridges and a near layer built from
//! real block cells (grass, dirt, stone, water, trees). Used for the Home
//! hero banner, tool backdrops and per-world thumbnails (seeded by the
//! world's name, so each world gets its own stable scene).

use crate::blocks::{Material, texture};
use crate::image::{Image, Rgba, Rng, fbm1, hash2, hex, mix, shade};

/// Scene presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scene {
    Day,
    Sunset,
    Night,
    Nether,
    End,
    Snow,
    Cave,
}

impl Scene {
    pub const ALL: [Scene; 7] = [Scene::Day, Scene::Sunset, Scene::Night, Scene::Nether, Scene::End, Scene::Snow, Scene::Cave];

    pub fn name(self) -> &'static str {
        match self {
            Scene::Day => "day",
            Scene::Sunset => "sunset",
            Scene::Night => "night",
            Scene::Nether => "nether",
            Scene::End => "end",
            Scene::Snow => "snow",
            Scene::Cave => "cave",
        }
    }
}

struct Look {
    sky_top: Rgba,
    sky_horizon: Rgba,
    far: Rgba,
    surface: Material,
    under: Material,
    deep: Material,
    fluid: Option<(Material, Rgba)>,
    trees: bool,
    light: f32,
}

fn look(s: Scene) -> Look {
    match s {
        Scene::Day => Look { sky_top: hex(0x3d7fd9), sky_horizon: hex(0xa9d4f5), far: hex(0x6f9fc0), surface: Material::GrassTop, under: Material::Dirt, deep: Material::Stone, fluid: Some((Material::Water, hex(0x3a6fdc))), trees: true, light: 1.0 },
        Scene::Sunset => Look { sky_top: hex(0x2b2f6b), sky_horizon: hex(0xf59a52), far: hex(0x6a4a6e), surface: Material::GrassTop, under: Material::Dirt, deep: Material::Stone, fluid: Some((Material::Water, hex(0x5a4fa0))), trees: true, light: 0.78 },
        Scene::Night => Look { sky_top: hex(0x070b1c), sky_horizon: hex(0x1d2b52), far: hex(0x1a2440), surface: Material::GrassTop, under: Material::Dirt, deep: Material::Stone, fluid: Some((Material::Water, hex(0x1a2c66))), trees: true, light: 0.42 },
        Scene::Nether => Look { sky_top: hex(0x1c0606), sky_horizon: hex(0x6b1a0e), far: hex(0x4a1210), surface: Material::Netherrack, under: Material::Netherrack, deep: Material::Netherrack, fluid: Some((Material::GoldBlock, hex(0xff7a1a))), trees: false, light: 0.85 },
        Scene::End => Look { sky_top: hex(0x0b0614), sky_horizon: hex(0x2a1a40), far: hex(0x2a1f3c), surface: Material::EndStone, under: Material::EndStone, deep: Material::Obsidian, fluid: None, trees: false, light: 0.9 },
        Scene::Snow => Look { sky_top: hex(0x6d8fb8), sky_horizon: hex(0xdbe8f2), far: hex(0x9fb4c8), surface: Material::Snow, under: Material::Dirt, deep: Material::Stone, fluid: Some((Material::Glass, hex(0x9fd0ea))), trees: true, light: 1.0 },
        Scene::Cave => Look { sky_top: hex(0x0a0b0e), sky_horizon: hex(0x15171c), far: hex(0x1d1f26), surface: Material::Stone, under: Material::Stone, deep: Material::Deepslate, fluid: Some((Material::Water, hex(0x203a7a))), trees: false, light: 0.6 },
    }
}

/// 4×4 Bayer matrix (ordered dithering for banded skies).
const BAYER: [[f32; 4]; 4] = [[0.0, 8.0, 2.0, 10.0], [12.0, 4.0, 14.0, 6.0], [3.0, 11.0, 1.0, 9.0], [15.0, 7.0, 13.0, 5.0]];

/// Generate a `w × h` scene. `cell` is the block size in pixels (4–8 reads
/// best). Deterministic for `(scene, seed, w, h, cell)`.
pub fn landscape(scene: Scene, seed: u64, w: u32, h: u32, cell: u32) -> Image {
    landscape_with(scene, seed, w, h, cell, &|_| None)
}

/// [`landscape`] with block textures from `tex` where it has them (e.g.
/// the user's installed resource pack), procedural ones otherwise.
pub fn landscape_with(scene: Scene, seed: u64, w: u32, h: u32, cell: u32, tex: &dyn Fn(Material) -> Option<Image>) -> Image {
    let texture = |m: Material| tex(m).unwrap_or_else(|| texture(m));
    let lk = look(scene);
    let mut img = Image::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let mut rng = Rng::new(seed ^ (scene as u64 * 0x51));

    // Sky: 10 color bands, dithered between neighbours.
    let bands = 10.0;
    for y in 0..hi {
        let t = y as f32 / (hi as f32 * 0.8);
        for x in 0..wi {
            let d = BAYER[(y & 3) as usize][(x & 3) as usize] / 16.0;
            let q = ((t * bands + d - 0.5).floor() / bands).clamp(0.0, 1.0);
            img.set(x, y, mix(lk.sky_top, lk.sky_horizon, q));
        }
    }

    // Stars (night / end / nether embers).
    if matches!(scene, Scene::Night | Scene::End | Scene::Nether) {
        let n = (w * h / 260) as i32;
        for _ in 0..n {
            let (x, y) = (rng.range(0, wi), rng.range(0, hi * 2 / 3));
            let c = if scene == Scene::Nether { hex(0xff9a3a) } else { hex(0xe8ecff) };
            img.set(x, y, [c[0], c[1], c[2], 120 + rng.range(0, 135) as u8]);
        }
    }

    // Celestial body: a square sun/moon with a stepped halo.
    let (sx, sy, size, core, halo) = match scene {
        Scene::Day | Scene::Snow => (wi * 3 / 4, hi / 6, cell as i32 * 3, hex(0xfff3b0), hex(0xfff8d8)),
        Scene::Sunset => (wi / 3, hi * 2 / 5, cell as i32 * 5, hex(0xffcf6a), hex(0xffb070)),
        Scene::Night => (wi / 5, hi / 7, cell as i32 * 3, hex(0xe6ecf5), hex(0x8090c0)),
        Scene::End => (wi * 2 / 3, hi / 5, cell as i32 * 2, hex(0xd6b8ff), hex(0x7d58c0)),
        Scene::Nether | Scene::Cave => (0, 0, 0, hex(0), hex(0)),
    };
    if size > 0 {
        for ring in (1..=3).rev() {
            let r = size / 2 + ring * cell as i32 / 2;
            let a = 30 + (3 - ring) as u8 * 22;
            img.fill_rect(sx - r, sy - r, 2 * r, 2 * r, [halo[0], halo[1], halo[2], a]);
        }
        img.fill_rect(sx - size / 2, sy - size / 2, size, size, core);
        if scene == Scene::Night {
            // Moon craters.
            img.fill_rect(sx - size / 4, sy - size / 6, cell as i32 / 2 + 1, cell as i32 / 2 + 1, hex(0xc4ccd8));
            img.fill_rect(sx + size / 8, sy + size / 8, cell as i32 / 2, cell as i32 / 2, hex(0xc4ccd8));
        }
    }

    // Blocky clouds (two layers).
    if matches!(scene, Scene::Day | Scene::Sunset | Scene::Snow | Scene::Night) {
        let cloud = match scene {
            Scene::Sunset => [0xff, 0xc8, 0xa8, 200],
            Scene::Night => [0x40, 0x4c, 0x70, 150],
            _ => [0xff, 0xff, 0xff, 215],
        };
        for layer in 0..2 {
            let cy = hi / 9 + layer * hi / 8;
            let mut x = -rng.range(0, 40);
            while x < wi {
                let cw = rng.range(4, 12) * cell as i32;
                let ch = cell as i32 * (1 + rng.range(0, 2));
                img.fill_rect(x, cy + rng.range(-2, 3) * cell as i32 / 2, cw, ch, cloud);
                img.fill_rect(x + cell as i32, cy - cell as i32 / 2 * (layer + 1), cw - 2 * cell as i32, cell as i32 / 2 * (layer + 1), cloud);
                x += cw + rng.range(6, 20) * cell as i32;
            }
        }
    }

    // Far + mid ridges with aerial perspective.
    for (layer, (base, amp, fade)) in [(0.50f32, 0.18f32, 0.55f32), (0.60, 0.16, 0.30)].iter().enumerate() {
        let col = mix(lk.far, lk.sky_horizon, *fade);
        for x in 0..wi {
            // Step the ridge to whole cells for a blocky silhouette.
            let xs = (x / cell as i32) as f32;
            let n = fbm1(seed + layer as u64 * 101 + scene as u64, xs * 0.09, 4);
            let top = ((base - amp * (n - 0.3)) * hi as f32) as i32;
            let top = top / cell as i32 * cell as i32;
            for y in top.max(0)..hi {
                img.set(x, y, col);
            }
        }
    }

    // Near layer from block cells.
    let c = cell as i32;
    let cols = wi / c + 1;
    let rows = hi / c + 1;
    let sea = (rows as f32 * 0.70) as i32;
    let tex_surface = texture(lk.surface);
    let tex_under = texture(lk.under);
    let tex_deep = texture(lk.deep);
    let fluid = lk.fluid.map(|(m, tint)| (texture(m), tint));
    let mut heights = Vec::with_capacity(cols as usize);
    for i in 0..cols {
        let n = fbm1(seed ^ 0x5eed ^ scene as u64, i as f32 * 0.06, 4);
        let hill = (rows as f32 * (0.60 + (n - 0.5) * 0.62)) as i32;
        heights.push(hill.clamp(rows / 3, rows - 2));
    }
    let draw_cell = |img: &mut Image, cx: i32, cy: i32, tex: &Image, light: f32| {
        for py in 0..c {
            for px in 0..c {
                let t = tex.get(px * tex.w as i32 / c, py * tex.h as i32 / c);
                img.set(cx * c + px, cy * c + py, shade(t, light));
            }
        }
    };
    // Ceilings (Nether / caves): hanging rock with stalactites.
    if matches!(scene, Scene::Nether | Scene::Cave) {
        let ceil_tex = if scene == Scene::Nether { texture(Material::Netherrack) } else { texture(Material::Deepslate) };
        for i in 0..cols {
            let n = fbm1(seed ^ 0xce11, i as f32 * 0.11, 3);
            let depth = 1 + (n * rows as f32 * 0.32) as i32;
            for r in 0..depth {
                draw_cell(&mut img, i, r, &ceil_tex, lk.light * (0.55 + 0.1 * (r == depth - 1) as i32 as f32));
            }
            // Glowstone clusters / ore glints on the ceiling edge.
            if hash2(seed, i, 31) > 0.82 {
                let glow_mat = if scene == Scene::Nether { Material::GoldBlock } else { Material::DiamondOre };
                draw_cell(&mut img, i, depth, &texture(glow_mat), 1.0);
                if scene == Scene::Nether {
                    glow(&mut img, i * c + c / 2, (depth * c) + c / 2, c * 3, [255, 210, 120]);
                }
            }
        }
    }
    // End: obsidian pillars behind the near layer.
    if scene == Scene::End {
        let obs = texture(Material::Obsidian);
        for k in 0..5 {
            let i = (cols * (k * 2 + 1)) / 10 + rng.range(-1, 2);
            let tall = rows / 3 + rng.range(0, rows / 4);
            for r in (heights[i.clamp(0, cols - 1) as usize] - tall).max(1)..rows {
                draw_cell(&mut img, i, r, &obs, 0.8);
                draw_cell(&mut img, i + 1, r, &obs, 0.62);
            }
            let top = (heights[i.clamp(0, cols - 1) as usize] - tall).max(1);
            glow(&mut img, i * c + c, top * c - c, c * 2, [220, 160, 255]);
        }
    }
    for i in 0..cols {
        let top = heights[i as usize];
        let dirt = 3 + (hash2(seed, i, 3) * 2.0) as i32;
        for r in top..rows {
            let depth = r - top;
            let mut tex = if depth == 0 { &tex_surface } else if depth < dirt { &tex_under } else { &tex_deep };
            // Underwater beds are sand/dirt, never grass or snow.
            if depth == 0 && top > sea && matches!(lk.surface, Material::GrassTop | Material::Snow) {
                tex = &tex_under;
            }
            // Cells get darker with depth (ambient occlusion feel).
            let l = lk.light * (1.0 - (depth as f32 * 0.05).min(0.42));
            draw_cell(&mut img, i, r, tex, l);
            // Grass blades hang over the side of a step down.
            if depth == 0 && tex == &tex_surface && i > 0 && heights[(i - 1) as usize] > top {
                img.fill_rect(i * c - 1, r * c, 1, c / 2, shade([0x4f, 0x94, 0x33, 255], lk.light));
            }
        }
        // Fluid fills valleys below sea level.
        if let Some((ftex, tint)) = &fluid
            && top > sea
        {
            for r in sea..top {
                for py in 0..c {
                    for px in 0..c {
                        let t = ftex.get(px * ftex.w as i32 / c, py * ftex.h as i32 / c);
                        let mut col = mix(t, *tint, 0.45);
                        col[3] = if scene == Scene::Nether { 255 } else { 205 };
                        let surface = r == sea && py == 0;
                        let col = if surface { mix(col, hex(0xffffff), 0.35) } else { col };
                        img.blend(i * c + px, r * c + py, shade(col, lk.light.max(0.6)));
                    }
                }
            }
        }
        // Trees on dry surface cells.
        if lk.trees && top <= sea && i > 1 && i < cols - 2 && hash2(seed, i, 7) > 0.84 {
            let trunk = texture(Material::LogSide);
            let leaves = texture(if scene == Scene::Snow { Material::Snow } else { Material::Leaves });
            let th = 2 + (hash2(seed, i, 9) * 2.0) as i32;
            for k in 1..=th {
                draw_cell(&mut img, i, top - k, &trunk, lk.light * 0.9);
            }
            // Canopy: 3 wide × 2 tall plus a crown.
            for dy in 0..2 {
                for dx in -1i32..=1 {
                    draw_cell(&mut img, i + dx, top - th - dy, &leaves, lk.light * (0.95 + 0.05 * dy as f32));
                }
            }
            draw_cell(&mut img, i, top - th - 2, &leaves, lk.light * 1.05);
        }
        // Torches in the cave.
        if scene == Scene::Cave && hash2(seed, i, 41) > 0.9 {
            let (tx, ty) = (i * c + c / 2, top * c - c);
            img.fill_rect(tx, ty, (c / 4).max(1), c, hex(0x7f5d3b));
            img.fill_rect(tx, ty - c / 4, (c / 4).max(1), c / 4 + 1, hex(0xffd36a));
            glow(&mut img, tx, ty, c * 4, [255, 190, 100]);
        }
    }

    // Lava glow from below.
    if scene == Scene::Nether {
        for y in 0..hi {
            let a = ((y as f32 / hi as f32).powf(2.0) * 70.0) as u8;
            for x in 0..wi {
                img.blend(x, y, [255, 110, 30, a]);
            }
        }
    }
    img
}

/// Soft additive-looking radial glow (torches, glowstone, crystals).
fn glow(img: &mut Image, cx: i32, cy: i32, r: i32, rgb: [u8; 3]) {
    for y in cy - r..=cy + r {
        for x in cx - r..=cx + r {
            let d = (((x - cx) * (x - cx) + (y - cy) * (y - cy)) as f32).sqrt() / r as f32;
            if d < 1.0 {
                // Quantized falloff keeps the pixel-art banding.
                let a = (((1.0 - d) * 4.0).floor() / 4.0 * 70.0) as u8;
                img.blend(x, y, [rgb[0], rgb[1], rgb[2], a]);
            }
        }
    }
}

/// Bake a vertical fade to `color` over the bottom `frac` of the image
/// (lets UI text sit on top of the art).
pub fn fade_bottom(img: &mut Image, color: Rgba, frac: f32) {
    let h = img.h as i32;
    let start = (h as f32 * (1.0 - frac)) as i32;
    for y in start..h {
        let t = (y - start) as f32 / (h - 1 - start).max(1) as f32;
        let a = (t.powf(1.4) * 255.0) as u8;
        for x in 0..img.w as i32 {
            img.blend(x, y, [color[0], color[1], color[2], a]);
        }
    }
}

/// Stable scene + seed for a world (thumbnails for worlds without an icon).
pub fn scene_for(name: &str, bedrock: bool) -> (Scene, u64) {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    let scenes = if bedrock { [Scene::Day, Scene::Sunset, Scene::Snow, Scene::Night] } else { [Scene::Day, Scene::Sunset, Scene::Night, Scene::Snow] };
    (scenes[(h % 4) as usize], h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_render_opaque_and_differ() {
        let a = landscape(Scene::Day, 1, 160, 60, 4);
        assert_eq!(a.coverage(), 1.0, "sky fills everything");
        assert_eq!(a, landscape(Scene::Day, 1, 160, 60, 4), "deterministic");
        assert_ne!(a, landscape(Scene::Day, 2, 160, 60, 4), "seeded");
        for s in Scene::ALL {
            let img = landscape(s, 3, 96, 40, 4);
            assert_eq!((img.w, img.h), (96, 40));
        }
    }

    #[test]
    fn fade_and_scene_for() {
        let mut img = Image::filled(4, 10, hex(0xffffff));
        fade_bottom(&mut img, hex(0x000000), 0.5);
        assert_eq!(img.get(0, 0), hex(0xffffff));
        assert!(img.get(0, 9)[0] < 40);
        assert_eq!(scene_for("My World", false), scene_for("My World", false));
    }
}
