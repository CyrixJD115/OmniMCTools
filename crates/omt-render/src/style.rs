//! Map styles as a **post-process** over a rendered tile's base colors,
//! surface heights and surface blocks. Changing the style, the sun angle
//! or the contour step re-lights loaded tiles in milliseconds without
//! reading the world again (render the tiles with `relief: false`).
//!
//! * **Classic** — the north-neighbour relief of the UMT-style maps.
//! * **Shaded** — Lambert hillshade from a movable sun (azimuth/altitude)
//!   with an ambient term.
//! * **Topographic** — elevation color ramp, contour lines, hillshade;
//!   water keeps its own color.
//! * **Heightmap** — grayscale elevation.
//! * **Night** — moonlit ambient plus block light spreading from emissive
//!   surface blocks (torches, lanterns, lava, glowstone…), falling off one
//!   level per block like the game's light engine (a two-pass L1 distance
//!   transform, O(pixels)). Light does not cross tile edges.

use serde::{Deserialize, Serialize};

/// Map style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MapStyle {
    Classic,
    #[default]
    Shaded,
    Topographic,
    Heightmap,
    Night,
    Biomes,
}

impl MapStyle {
    pub const ALL: [MapStyle; 6] = [Self::Classic, Self::Shaded, Self::Topographic, Self::Heightmap, Self::Night, Self::Biomes];

    pub fn name(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Shaded => "Shaded",
            Self::Topographic => "Topographic",
            Self::Heightmap => "Heightmap",
            Self::Night => "Night",
            Self::Biomes => "Biomes",
        }
    }
}

impl std::fmt::Display for MapStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Style parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleParams {
    pub style: MapStyle,
    /// Sun direction, degrees clockwise from north (map up).
    pub azimuth: f32,
    /// Sun height above the horizon, degrees.
    pub altitude: f32,
    /// 0 = flat colors, 1 = full hillshade.
    pub strength: f32,
    /// Vertical exaggeration of the terrain for lighting.
    pub exaggeration: f32,
    /// Contour interval in blocks (Topographic), 0 = none.
    pub contour: i32,
    /// Night ambient brightness (0..1).
    pub night_ambient: f32,
}

impl Default for StyleParams {
    fn default() -> Self {
        Self { style: MapStyle::Shaded, azimuth: 315.0, altitude: 45.0, strength: 0.75, exaggeration: 1.0, contour: 16, night_ambient: 0.32 }
    }
}

/// One tile's inputs.
pub struct TileView<'a> {
    pub px: usize,
    /// Blocks per pixel edge.
    pub bpp: f32,
    pub rgba: &'a [u8],
    pub heights: &'a [i16],
    pub surface: &'a [u16],
    /// Surface biome ids into `names` (may be empty: no biome layer).
    pub biomes: &'a [u16],
    pub names: &'a [String],
}

/// Block light emitted by a surface block (0 = none), by name.
pub fn emission(name: &str) -> u8 {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    const FULL: [&str; 16] = [
        "torch", "lantern", "glowstone", "sea_lantern", "shroomlight", "lava", "fire", "campfire", "jack_o_lantern", "beacon", "end_rod", "froglight", "redstone_lamp", "respawn_anchor", "conduit", "lit_pumpkin",
    ];
    if n.contains("redstone_torch") || n == "magma" || n == "magma_block" || n.contains("crying_obsidian") || n.contains("soul_") {
        return 10;
    }
    if n.contains("candle") || n.contains("sea_pickle") || n.contains("glow_lichen") || n.contains("amethyst_cluster") {
        return 7;
    }
    if FULL.iter().any(|k| n.contains(k)) && !n.contains("unlit") {
        return 15;
    }
    0
}

fn is_water(name: &str) -> bool {
    name.ends_with("water") || name.contains("kelp") || name.contains("seagrass")
}

/// Biome map color (OMT's own palette, by biome family; unknown biomes get
/// a stable hashed color).
pub fn biome_color(name: &str) -> [u8; 3] {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let has = |k: &str| n.contains(k);
    if has("ocean") {
        return if has("deep") { [24, 52, 120] } else if has("warm") && !has("luke") { [24, 130, 175] } else if has("lukewarm") { [30, 105, 170] } else if has("frozen") || has("cold") { [70, 92, 172] } else { [38, 74, 162] };
    }
    let table: [(&str, [u8; 3]); 40] = [
        ("frozen_river", [150, 165, 230]),
        ("river", [52, 104, 222]),
        ("stony_shore", [128, 128, 120]),
        ("snowy_beach", [232, 232, 206]),
        ("beach", [226, 210, 150]),
        ("desert", [232, 202, 112]),
        ("eroded", [190, 92, 52]),
        ("badlands", [206, 112, 62]),
        ("savanna", [182, 170, 82]),
        ("sunflower", [158, 196, 82]),
        ("plains", [132, 182, 82]),
        ("meadow", [122, 204, 112]),
        ("cherry", [238, 168, 200]),
        ("pale_garden", [182, 182, 172]),
        ("birch", [112, 172, 92]),
        ("dark_forest", [48, 98, 40]),
        ("flower_forest", [96, 176, 84]),
        ("forest", [70, 140, 60]),
        ("snowy_taiga", [84, 124, 112]),
        ("taiga", [62, 112, 92]),
        ("bamboo", [92, 172, 42]),
        ("sparse_jungle", [104, 172, 62]),
        ("jungle", [48, 150, 30]),
        ("mangrove", [72, 102, 62]),
        ("swamp", [62, 92, 72]),
        ("grove", [180, 202, 192]),
        ("peaks", [214, 222, 236]),
        ("snowy", [228, 234, 244]),
        ("ice", [176, 214, 240]),
        ("windswept", [112, 132, 112]),
        ("mushroom", [172, 112, 172]),
        ("lush", [100, 192, 92]),
        ("dripstone", [140, 112, 92]),
        ("deep_dark", [20, 42, 52]),
        ("crimson", [172, 32, 42]),
        ("warped", [32, 142, 132]),
        ("soul_sand", [102, 82, 62]),
        ("basalt", [82, 82, 88]),
        ("nether", [152, 52, 42]),
        ("end", [196, 186, 228]),
    ];
    if let Some((_, c)) = table.iter().find(|(k, _)| has(k)) {
        return *c;
    }
    let h = n.bytes().fold(0xcbf2_9ce4u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    [60 + (h & 0x7f) as u8, 60 + ((h >> 8) & 0x7f) as u8, 60 + ((h >> 16) & 0x7f) as u8]
}

/// Elevation ramp (y → color), OMT's own cartographic palette.
fn topo_color(h: i32, water: bool) -> [f32; 3] {
    if water {
        return [0.25, 0.45, 0.80];
    }
    const STOPS: [(i32, [f32; 3]); 8] = [
        (-64, [0.20, 0.22, 0.28]),
        (0, [0.30, 0.34, 0.36]),
        (62, [0.36, 0.58, 0.32]),
        (80, [0.50, 0.70, 0.36]),
        (100, [0.80, 0.78, 0.45]),
        (130, [0.66, 0.50, 0.32]),
        (170, [0.58, 0.55, 0.52]),
        (220, [0.96, 0.97, 1.00]),
    ];
    if h <= STOPS[0].0 {
        return STOPS[0].1;
    }
    for w in STOPS.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if h <= b {
            let t = (h - a) as f32 / (b - a) as f32;
            return [ca[0] + (cb[0] - ca[0]) * t, ca[1] + (cb[1] - ca[1]) * t, ca[2] + (cb[2] - ca[2]) * t];
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// Apply a style; returns new RGBA (same size). Transparent pixels stay
/// transparent.
pub fn stylize(t: &TileView<'_>, p: &StyleParams) -> Vec<u8> {
    let n = t.px * t.px;
    let mut out = t.rgba.to_vec();
    if n == 0 || t.rgba.len() < n * 4 || t.heights.len() < n {
        return out;
    }
    let h_at = |x: i64, z: i64, fallback: i16| -> f32 {
        let x = x.clamp(0, t.px as i64 - 1) as usize;
        let z = z.clamp(0, t.px as i64 - 1) as usize;
        let h = t.heights[z * t.px + x];
        if h == i16::MIN { fallback as f32 } else { h as f32 }
    };
    let name_at = |i: usize| t.surface.get(i).and_then(|&s| t.names.get(s as usize)).map(String::as_str).unwrap_or("");
    // Sun vector (x east, z south, y up).
    let az = p.azimuth.to_radians();
    let alt = p.altitude.clamp(1.0, 90.0).to_radians();
    let sun = [az.sin() * alt.cos(), alt.sin(), -az.cos() * alt.cos()];
    let flat = sun[1];
    let light = if p.style == MapStyle::Night { night_light(t) } else { Vec::new() };
    let (mut hmin, mut hmax) = (i16::MAX, i16::MIN);
    if p.style == MapStyle::Heightmap {
        for &h in &t.heights[..n] {
            if h != i16::MIN {
                hmin = hmin.min(h);
                hmax = hmax.max(h);
            }
        }
        // Stable scale across tiles: the world's full Y range.
        hmin = hmin.min(-64);
        hmax = hmax.max(320);
    }
    for z in 0..t.px {
        for x in 0..t.px {
            let i = z * t.px + x;
            let o = i * 4;
            if t.rgba[o + 3] == 0 {
                continue;
            }
            let h0 = t.heights[i];
            let base = [t.rgba[o] as f32 / 255.0, t.rgba[o + 1] as f32 / 255.0, t.rgba[o + 2] as f32 / 255.0];
            let (xi, zi) = (x as i64, z as i64);
            // Height gradient in blocks per block.
            let dzdx = (h_at(xi + 1, zi, h0) - h_at(xi - 1, zi, h0)) / (2.0 * t.bpp) * p.exaggeration;
            let dzdz = (h_at(xi, zi + 1, h0) - h_at(xi, zi - 1, h0)) / (2.0 * t.bpp) * p.exaggeration;
            let nrm = {
                let v = [-dzdx, 1.0, -dzdz];
                let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                [v[0] / l, v[1] / l, v[2] / l]
            };
            let lambert = (nrm[0] * sun[0] + nrm[1] * sun[1] + nrm[2] * sun[2]).max(0.0);
            // Normalized so flat ground keeps its color.
            let shade = 1.0 + (lambert / flat.max(0.05) - 1.0) * p.strength;
            let shade = shade.clamp(0.25, 1.6);
            let water = is_water(name_at(i));
            let c = match p.style {
                MapStyle::Classic => {
                    let north = if z > 0 { t.heights[i - t.px] } else { i16::MIN };
                    let f = if north == i16::MIN || h0 == i16::MIN {
                        1.0
                    } else {
                        match h0.cmp(&north) {
                            std::cmp::Ordering::Greater => 1.12,
                            std::cmp::Ordering::Less => 0.82,
                            std::cmp::Ordering::Equal => 1.0,
                        }
                    };
                    [base[0] * f, base[1] * f, base[2] * f]
                }
                MapStyle::Shaded => {
                    let s = if water { 1.0 + (shade - 1.0) * 0.25 } else { shade };
                    [base[0] * s, base[1] * s, base[2] * s]
                }
                MapStyle::Topographic => {
                    let col = if water { base } else { topo_color(h0 as i32, false) };
                    let s = if water { 1.0 } else { shade };
                    let mut c = [col[0] * s, col[1] * s, col[2] * s];
                    if p.contour > 0 && !water && h0 != i16::MIN {
                        let band = |h: f32| (h / p.contour as f32).floor() as i32;
                        let b0 = band(h0 as f32);
                        if band(h_at(xi + 1, zi, h0)) != b0 || band(h_at(xi, zi + 1, h0)) != b0 {
                            let major = b0.rem_euclid(5) == 0;
                            let k = if major { 0.55 } else { 0.75 };
                            c = [c[0] * k, c[1] * k, c[2] * k];
                        }
                    }
                    c
                }
                MapStyle::Heightmap => {
                    let v = if h0 == i16::MIN { 0.0 } else { (h0 - hmin) as f32 / (hmax - hmin).max(1) as f32 };
                    let v = v * shade.clamp(0.7, 1.2);
                    [v, v, v]
                }
                MapStyle::Biomes => {
                    let b = t.biomes.get(i).copied().unwrap_or(u16::MAX);
                    match t.names.get(b as usize) {
                        Some(name) => {
                            let c = biome_color(name);
                            let s = shade.clamp(0.6, 1.35);
                            [c[0] as f32 / 255.0 * s, c[1] as f32 / 255.0 * s, c[2] as f32 / 255.0 * s]
                        }
                        None => [base[0] * 0.35, base[1] * 0.35, base[2] * 0.35],
                    }
                }
                MapStyle::Night => {
                    let bl = light.get(i).copied().unwrap_or(0) as f32 / 15.0;
                    let amb = p.night_ambient * (0.6 + 0.4 * shade.min(1.2));
                    // Moonlight is cool, block light warm.
                    let moon = [amb * 0.75, amb * 0.85, amb * 1.25];
                    let warm = [1.0 * bl, 0.82 * bl, 0.55 * bl];
                    let glow = |k: usize| base[k] * (moon[k] + warm[k] * 1.1);
                    let mut c = [glow(0), glow(1), glow(2)];
                    if emission(name_at(i)) > 0 {
                        c = [base[0].max(0.9), base[1].max(0.75), base[2].max(0.4)];
                    }
                    c
                }
            };
            for k in 0..3 {
                out[o + k] = (c[k] * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Block-light map (0..15 per pixel) from emissive surface blocks.
fn night_light(t: &TileView<'_>) -> Vec<u8> {
    let n = t.px * t.px;
    let mut l = vec![0u8; n];
    let mut ids: Vec<u8> = vec![0; t.names.len()];
    for (k, name) in t.names.iter().enumerate() {
        ids[k] = emission(name);
    }
    for (i, slot) in l.iter_mut().enumerate() {
        if let Some(&s) = t.surface.get(i) {
            *slot = ids.get(s as usize).copied().unwrap_or(0);
        }
    }
    // Light drops one level per block; a pixel spans `bpp` blocks.
    let decay = t.bpp.max(1.0).round() as u8;
    let w = t.px;
    for z in 0..w {
        for x in 0..w {
            let i = z * w + x;
            let mut v = l[i];
            if x > 0 {
                v = v.max(l[i - 1].saturating_sub(decay));
            }
            if z > 0 {
                v = v.max(l[i - w].saturating_sub(decay));
            }
            l[i] = v;
        }
    }
    for z in (0..w).rev() {
        for x in (0..w).rev() {
            let i = z * w + x;
            let mut v = l[i];
            if x + 1 < w {
                v = v.max(l[i + 1].saturating_sub(decay));
            }
            if z + 1 < w {
                v = v.max(l[i + w].saturating_sub(decay));
            }
            l[i] = v;
        }
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(px: usize, heights: Vec<i16>, surface: Vec<u16>, names: Vec<String>) -> (Vec<u8>, Vec<i16>, Vec<u16>, Vec<String>) {
        let rgba = [120u8, 160, 90, 255].repeat(px * px);
        (rgba, heights, surface, names)
    }

    #[test]
    fn hillshade_lights_sun_facing_slopes() {
        let px = 8;
        // Height rises to the east: slope faces west.
        let heights: Vec<i16> = (0..px * px).map(|i| ((i % px) * 4) as i16 + 64).collect();
        let (rgba, h, s, n) = tile(px, heights, vec![0; px * px], vec!["minecraft:stone".into()]);
        let v = TileView { px, bpp: 1.0, rgba: &rgba, heights: &h, surface: &s, biomes: &[], names: &n };
        let west_sun = stylize(&v, &StyleParams { azimuth: 270.0, ..Default::default() });
        let east_sun = stylize(&v, &StyleParams { azimuth: 90.0, ..Default::default() });
        let i = (4 * px + 4) * 4;
        assert!(west_sun[i] > east_sun[i], "{} vs {}", west_sun[i], east_sun[i]);
        // Flat terrain keeps its base color.
        let (rgba, h, s, n) = tile(px, vec![64; px * px], vec![0; px * px], vec!["minecraft:stone".into()]);
        let v = TileView { px, bpp: 1.0, rgba: &rgba, heights: &h, surface: &s, biomes: &[], names: &n };
        let out = stylize(&v, &StyleParams::default());
        assert!((out[i] as i32 - 120).abs() <= 1);
    }

    #[test]
    fn night_glows_around_torches() {
        let px = 16;
        let mut surface = vec![0u16; px * px];
        surface[8 * px + 8] = 1;
        let (rgba, h, s, n) = tile(px, vec![64; px * px], surface, vec!["minecraft:grass_block".into(), "minecraft:torch".into()]);
        let v = TileView { px, bpp: 1.0, rgba: &rgba, heights: &h, surface: &s, biomes: &[], names: &n };
        let out = stylize(&v, &StyleParams { style: MapStyle::Night, ..Default::default() });
        let lum = |x: usize, z: usize| {
            let o = (z * px + x) * 4;
            out[o] as u32 + out[o + 1] as u32 + out[o + 2] as u32
        };
        assert!(lum(9, 8) > lum(14, 8), "near brighter than far");
        assert!(lum(14, 8) >= lum(0, 0));
        assert_eq!(emission("minecraft:soul_lantern"), 10);
        assert_eq!(emission("minecraft:lantern"), 15);
        assert_eq!(emission("minecraft:stone"), 0);
        assert_eq!(biome_color("minecraft:desert"), [232, 202, 112]);
        assert_ne!(biome_color("minecraft:deep_ocean"), biome_color("minecraft:ocean"));
        assert_eq!(biome_color("mod:unknown"), biome_color("mod:unknown"), "stable fallback");
    }

    #[test]
    fn topographic_draws_contours_and_keeps_transparency() {
        let px = 8;
        let heights: Vec<i16> = (0..px * px).map(|i| ((i % px) * 3) as i16 + 60).collect();
        let (mut rgba, h, s, n) = tile(px, heights, vec![0; px * px], vec!["minecraft:grass_block".into()]);
        rgba[3] = 0;
        let v = TileView { px, bpp: 1.0, rgba: &rgba, heights: &h, surface: &s, biomes: &[], names: &n };
        let out = stylize(&v, &StyleParams { style: MapStyle::Topographic, contour: 4, ..Default::default() });
        assert_eq!(out[3], 0);
        let row: Vec<u8> = (0..px).map(|x| out[(3 * px + x) * 4 + 1]).collect();
        assert!(row.windows(2).any(|w| w[0].abs_diff(w[1]) > 20), "contour lines vary the row: {row:?}");
    }
}
