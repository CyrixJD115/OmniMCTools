//! Per-chunk top-down rendering (UMT's Chunk Locator / pruner viewer look:
//! block textures' colors, biome tinting, heightmap + water shading, and the
//! Slicer that hides everything above a Y level).

use omt_world::{BedrockBiomeSlot, Chunk};

use crate::palette::{BiomeColors, BlockColor, Palette, Tint};

/// Rendering knobs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOptions {
    /// Slicer: ignore every block above this Y (inclusive top).
    pub max_y: Option<i32>,
    /// Lowest Y scanned (world floor).
    pub min_y: i32,
    pub biome_tint: bool,
    /// Darken deeper water.
    pub water_shading: bool,
    /// Bake north-neighbour relief shading into the colors (UMT look).
    /// Off for viewers that light the map themselves (see [`crate::style`]).
    pub relief: bool,
    /// Underground mode: below the surface (or slicer), skip down through
    /// the first solid layer and draw the floor of the first cave beneath.
    pub cave: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { max_y: None, min_y: -64, biome_tint: true, water_shading: true, relief: true, cave: false }
    }
}

impl RenderOptions {
    /// Sensible defaults per dimension: the Nether gets a slicer below its
    /// bedrock roof, as a map of the roof shows nothing.
    pub fn for_dimension(dim: u8) -> Self {
        match dim {
            1 => Self { max_y: Some(100), min_y: 0, ..Self::default() },
            2 => Self { min_y: 0, ..Self::default() },
            _ => Self::default(),
        }
    }
}

/// One rendered chunk: 16×16 RGBA (row-major, z rows, x columns), the
/// surface height per column (`i32::MIN` = nothing), and the surface
/// block per column for click-identify (an id into the chunk-local
/// `names` table; [`NO_SURFACE`] = none — no per-column string clones).
#[derive(Debug, Clone)]
pub struct ChunkRender {
    pub x: i32,
    pub z: i32,
    pub rgba: Vec<u8>,
    pub heights: [i32; 256],
    pub surface: [u16; 256],
    /// Biome at the surface per column (id into `names`; [`NO_SURFACE`] =
    /// unknown).
    pub biome: [u16; 256],
    pub names: Vec<String>,
}

/// `ChunkRender::surface` value for "no surface block".
pub const NO_SURFACE: u16 = u16::MAX;

impl ChunkRender {
    pub fn pixel(&self, lx: usize, lz: usize) -> [u8; 4] {
        let i = (lz * 16 + lx) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// Surface block name of column `i` (`lz * 16 + lx`).
    pub fn surface_name(&self, i: usize) -> Option<&str> {
        self.names.get(*self.surface.get(i)? as usize).map(String::as_str)
    }

    /// Surface biome name of column `i`.
    pub fn biome_name(&self, i: usize) -> Option<&str> {
        self.names.get(*self.biome.get(i)? as usize).map(String::as_str)
    }

    /// Box-average of the chunk's opaque pixels (LOD sampling). `None` when
    /// nothing rendered.
    pub fn average(&self) -> Option<[u8; 4]> {
        let (mut acc, mut n) = ([0u32; 3], 0u32);
        for px in self.rgba.as_chunks::<4>().0 {
            if px[3] != 0 {
                for i in 0..3 {
                    acc[i] += px[i] as u32;
                }
                n += 1;
            }
        }
        (n > 0).then(|| [(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8, 255])
    }

    /// Mean surface height (LOD relief shading), if any column rendered.
    pub fn mean_height(&self) -> Option<i32> {
        let (mut sum, mut n) = (0i64, 0i64);
        for &h in &self.heights {
            if h != i32::MIN {
                sum += h as i64;
                n += 1;
            }
        }
        (n > 0).then(|| (sum / n) as i32)
    }
}

fn mul(c: [u8; 4], tint: u32) -> [u8; 4] {
    let ch = |v: u8, s: u32| ((v as u32 * ((tint >> s) & 0xff)) / 255) as u8;
    [ch(c[0], 16), ch(c[1], 8), ch(c[2], 0), c[3]]
}

fn tinted(color: BlockColor, biome: BiomeColors, enabled: bool) -> [u8; 4] {
    if !enabled {
        return match color.tint {
            Tint::None => color.rgba,
            Tint::Grass => mul(color.rgba, BiomeColors::DEFAULT.grass),
            Tint::Foliage => mul(color.rgba, BiomeColors::DEFAULT.foliage),
            Tint::Water => mul(color.rgba, BiomeColors::DEFAULT.water),
        };
    }
    match color.tint {
        Tint::None => color.rgba,
        Tint::Grass => mul(color.rgba, biome.grass),
        Tint::Foliage => mul(color.rgba, biome.foliage),
        Tint::Water => mul(color.rgba, biome.water),
    }
}

/// Biome name at a block of the chunk (both editions).
pub fn biome_at<'p>(chunk: &'p Chunk, palette: &'p Palette, lx: usize, y: i32, lz: usize) -> Option<&'p str> {
    let sy = y.div_euclid(16) as i8;
    let iy = y.rem_euclid(16) as usize;
    if let Some(cells) = chunk.java_biomes.get(&sy) {
        return cells.at(lx, iy, lz);
    }
    let slots = chunk.bedrock_biomes.as_ref()?;
    // Slot i covers Y i*16-64; NonExistent repeats the slot below.
    let mut slot = ((y + 64).div_euclid(16)).clamp(0, slots.len() as i32 - 1) as usize;
    loop {
        match slots.get(slot)? {
            BedrockBiomeSlot::Storage { palette: ids, indices, .. } => {
                let id = match indices {
                    // XZY on disk: x*256 + z*16 + y
                    Some(ix) => ids.get(*ix.get(lx * 256 + lz * 16 + iy)? as usize)?,
                    None => ids.first()?,
                };
                return palette.bedrock_biome_name(*id);
            }
            BedrockBiomeSlot::NonExistent if slot > 0 => slot -= 1,
            BedrockBiomeSlot::NonExistent => return None,
        }
    }
}

/// Biome tint memo key: Java cells (section, palette index) or a Bedrock
/// biome id — the biome is resolved once per distinct value, and only for
/// blocks that actually take a tint.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum BiomeKey {
    Java(i8, u16),
    Bedrock(i32),
    Default,
}

fn biome_key(chunk: &Chunk, lx: usize, y: i32, lz: usize) -> BiomeKey {
    let sy = y.div_euclid(16) as i8;
    let iy = y.rem_euclid(16) as usize;
    if let Some(cells) = chunk.java_biomes.get(&sy) {
        // Java biome cells are 4×4×4.
        let i = (iy / 4) * 16 + (lz / 4) * 4 + lx / 4;
        // Same fallback as `BiomeCells::at`: a missing cell reads palette 0.
        return BiomeKey::Java(sy, cells.cells.get(i).copied().unwrap_or(0));
    }
    let Some(slots) = chunk.bedrock_biomes.as_ref() else { return BiomeKey::Default };
    let mut slot = ((y + 64).div_euclid(16)).clamp(0, slots.len() as i32 - 1) as usize;
    loop {
        match slots.get(slot) {
            Some(BedrockBiomeSlot::Storage { palette: ids, indices, .. }) => {
                let id = match indices {
                    Some(ix) => ix.get(lx * 256 + lz * 16 + iy).and_then(|&i| ids.get(i as usize)),
                    None => ids.first(),
                };
                return id.map_or(BiomeKey::Default, |&id| BiomeKey::Bedrock(id));
            }
            Some(BedrockBiomeSlot::NonExistent) if slot > 0 => slot -= 1,
            _ => return BiomeKey::Default,
        }
    }
}

fn biome_colors(chunk: &Chunk, palette: &Palette, key: BiomeKey) -> BiomeColors {
    match key {
        BiomeKey::Java(sy, p) => chunk
            .java_biomes
            .get(&sy)
            .and_then(|c| c.palette.get(p as usize))
            .map_or(BiomeColors::DEFAULT, |n| palette.biome(n)),
        BiomeKey::Bedrock(id) => palette.bedrock_biome_name(id).map_or(BiomeColors::DEFAULT, |n| palette.biome(n)),
        BiomeKey::Default => BiomeColors::DEFAULT,
    }
}

/// Render one chunk column top-down.
pub fn render_chunk(chunk: &Chunk, palette: &Palette, opts: &RenderOptions) -> ChunkRender {
    let mut rgba = vec![0u8; 16 * 16 * 4];
    let mut heights = [i32::MIN; 256];
    let mut surface = [NO_SURFACE; 256];
    let mut names: Vec<String> = Vec::new();
    let top_slice = chunk.slices.keys().next_back().copied().map(|s| s as i32 * 16 + 15);
    let Some(top) = top_slice else {
        return ChunkRender { x: chunk.x, z: chunk.z, rgba, heights, surface, biome: [NO_SURFACE; 256], names };
    };
    let start = opts.max_y.map_or(top, |m| m.min(top));
    // Per-slice color caches (palette index → color), built lazily, plus
    // per-slice palette index → chunk-local surface name id.
    let mut caches: std::collections::HashMap<i8, Vec<BlockColor>> = std::collections::HashMap::new();
    let mut name_ids: std::collections::HashMap<(i8, u16), u16> = std::collections::HashMap::new();
    let mut biomes: std::collections::HashMap<BiomeKey, BiomeColors> = std::collections::HashMap::new();

    for lz in 0..16 {
        for lx in 0..16 {
            let mut acc = [0f32; 3];
            let mut acc_a = 0f32;
            let mut water_seen = 0u32;
            let mut y = start;
            // Cave mode phases: 0 = above/in the first solid layer's
            // approach, 1 = inside rock, 2 = in the cave air (render from
            // the next solid block down).
            let mut phase = if opts.cave { 0u8 } else { 2 };
            while y >= opts.min_y && acc_a < 0.985 {
                let sy = y.div_euclid(16) as i8;
                let Some(slice) = chunk.slices.get(&sy) else {
                    // Missing section = all air: skip to the one below
                    // (in cave mode, air below rock is the cave).
                    if phase == 1 {
                        phase = 2;
                    }
                    y = sy as i32 * 16 - 1;
                    continue;
                };
                let cache = caches
                    .entry(sy)
                    .or_insert_with(|| slice.palette.iter().map(|b| palette.block(b)).collect());
                let idx = (y.rem_euclid(16) as usize) * 256 + lz * 16 + lx;
                let pi = *slice.indices.get(idx).unwrap_or(&0) as usize;
                let color = cache.get(pi).copied().unwrap_or(BlockColor::CLEAR);
                if phase < 2 {
                    let solid = !color.is_clear() && color.rgba[3] > 76 && color.tint != Tint::Water;
                    match (phase, solid) {
                        (0, true) => phase = 1,
                        (1, false) => phase = 2,
                        _ => {}
                    }
                    y -= 1;
                    continue;
                }
                if !color.is_clear() {
                    let is_water = color.tint == Tint::Water;
                    let c = if color.tint == Tint::None {
                        color.rgba
                    } else {
                        let key = biome_key(chunk, lx, y, lz);
                        let biome = *biomes.entry(key).or_insert_with(|| biome_colors(chunk, palette, key));
                        tinted(color, biome, opts.biome_tint)
                    };
                    let mut a = c[3] as f32 / 255.0;
                    if is_water {
                        // Surface water half-covers; each deeper block adds
                        // a little — shallows stay readable, depths darken.
                        a = if water_seen == 0 { 0.5 } else if opts.water_shading { 0.12 } else { 0.0 };
                        water_seen += 1;
                    }
                    if heights[lz * 16 + lx] == i32::MIN && (a > 0.3 || is_water) {
                        heights[lz * 16 + lx] = y;
                        if let Some(state) = slice.palette.get(pi) {
                            let id = *name_ids.entry((sy, pi as u16)).or_insert_with(|| {
                                names.push(state.name.clone());
                                (names.len() - 1) as u16
                            });
                            surface[lz * 16 + lx] = id;
                        }
                    }
                    let w = (1.0 - acc_a) * a;
                    for i in 0..3 {
                        acc[i] += w * c[i] as f32;
                    }
                    acc_a += w;
                    if water_seen > 24 {
                        break;
                    }
                }
                y -= 1;
            }
            let o = (lz * 16 + lx) * 4;
            if acc_a > 0.0 {
                // Normalize partially-covered columns (e.g. over the void).
                for i in 0..3 {
                    rgba[o + i] = (acc[i] / acc_a).round().clamp(0.0, 255.0) as u8;
                }
                rgba[o + 3] = 255;
            }
        }
    }
    // Surface biome per column (interned with the block names).
    let mut biome = [NO_SURFACE; 256];
    let mut biome_ids: std::collections::HashMap<String, u16> = std::collections::HashMap::new();
    for lz in 0..16 {
        for lx in 0..16 {
            let h = heights[lz * 16 + lx];
            if h == i32::MIN {
                continue;
            }
            if let Some(name) = biome_at(chunk, palette, lx, h, lz) {
                let id = match biome_ids.get(name) {
                    Some(&id) => id,
                    None => {
                        names.push(name.to_string());
                        let id = (names.len() - 1) as u16;
                        biome_ids.insert(name.to_string(), id);
                        id
                    }
                };
                biome[lz * 16 + lx] = id;
            }
        }
    }
    ChunkRender { x: chunk.x, z: chunk.z, rgba, heights, surface, biome, names }
}

/// Map-style height shading: a column higher than its northern neighbor
/// is lightened, lower is darkened. `north` is the southmost height row of
/// the chunk to the north (when loaded).
pub fn shade(render: &mut ChunkRender, north: Option<&[i32; 256]>) {
    for lz in 0..16 {
        for lx in 0..16 {
            let h = render.heights[lz * 16 + lx];
            if h == i32::MIN {
                continue;
            }
            let n = if lz > 0 {
                render.heights[(lz - 1) * 16 + lx]
            } else {
                north.map_or(i32::MIN, |nh| nh[15 * 16 + lx])
            };
            if n == i32::MIN {
                continue;
            }
            let factor = match h.cmp(&n) {
                std::cmp::Ordering::Greater => 1.12,
                std::cmp::Ordering::Less => 0.82,
                std::cmp::Ordering::Equal => 1.0,
            };
            let o = (lz * 16 + lx) * 4;
            for i in 0..3 {
                render.rgba[o + i] = (render.rgba[o + i] as f32 * factor).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omt_world::{BiomeCells, BlockState, Slice};

    fn st(n: &str) -> BlockState {
        BlockState { name: n.into(), properties: vec![] }
    }

    fn chunk_with(blocks: &[(i32, &str)]) -> Chunk {
        // Fill a full column stack at every x/z with the given Y → block.
        let mut chunk = Chunk { x: 0, z: 0, dim: 0, ..Default::default() };
        let mut palette = vec![st("minecraft:air")];
        let mut indices = vec![0u16; 4096];
        for (y, name) in blocks {
            palette.push(st(name));
            let p = (palette.len() - 1) as u16;
            for i in 0..256 {
                indices[(*y as usize) * 256 + i] = p;
            }
        }
        chunk.slices.insert(0, Slice { palette, indices });
        chunk
    }

    #[test]
    fn surface_height_and_slicer() {
        let p = Palette::embedded();
        let c = chunk_with(&[(0, "minecraft:stone"), (5, "minecraft:sand")]);
        let r = render_chunk(&c, p, &RenderOptions::default());
        assert_eq!(r.heights[0], 5);
        assert_eq!(r.surface_name(0), Some("minecraft:sand"));
        assert_eq!(r.pixel(0, 0)[3], 255);
        // Slicer below the sand exposes the stone.
        let sliced = render_chunk(&c, p, &RenderOptions { max_y: Some(4), ..Default::default() });
        assert_eq!(sliced.heights[0], 0);
        assert_eq!(sliced.surface_name(0), Some("minecraft:stone"));
        assert_ne!(r.pixel(3, 3), sliced.pixel(3, 3));
    }

    #[test]
    fn grass_takes_biome_tint() {
        let p = Palette::embedded();
        let mut plains = chunk_with(&[(3, "minecraft:grass_block")]);
        plains.java_biomes.insert(0, BiomeCells { palette: vec!["minecraft:plains".into()], cells: vec![0; 64] });
        let mut swamp = plains.clone();
        swamp.java_biomes.insert(0, BiomeCells { palette: vec!["minecraft:swamp".into()], cells: vec![0; 64] });
        let a = render_chunk(&plains, p, &RenderOptions::default()).pixel(0, 0);
        let b = render_chunk(&swamp, p, &RenderOptions::default()).pixel(0, 0);
        assert_ne!(a, b, "biome tint changes grass color");
        assert!(a[1] > a[0] && a[1] > a[2], "plains grass is green: {a:?}");
    }

    #[test]
    fn deep_water_is_darker_and_shading_works() {
        let p = Palette::embedded();
        let shallow = chunk_with(&[(0, "minecraft:sand"), (1, "minecraft:water")]);
        let mut deep_blocks = vec![(0, "minecraft:sand")];
        for y in 1..15 {
            deep_blocks.push((y, "minecraft:water"));
        }
        let deep = chunk_with(&deep_blocks);
        let s = render_chunk(&shallow, p, &RenderOptions::default()).pixel(0, 0);
        let d = render_chunk(&deep, p, &RenderOptions::default()).pixel(0, 0);
        let lum = |c: [u8; 4]| c[0] as u32 + c[1] as u32 + c[2] as u32;
        assert!(lum(d) < lum(s), "deep {d:?} darker than shallow {s:?}");

        let mut r = render_chunk(&shallow, p, &RenderOptions::default());
        r.heights[16] = 10; // column (0,1) higher than its northern neighbor
        let before = r.pixel(0, 1);
        shade(&mut r, None);
        assert!(lum(r.pixel(0, 1)) > lum(before));
    }

    #[test]
    fn cave_mode_shows_the_cave_floor() {
        let p = Palette::embedded();
        // Grass surface at 12, stone roof 8..=11, air 5..=7, sand floor at 4.
        let c = chunk_with(&[(4, "minecraft:sand"), (8, "minecraft:stone"), (9, "minecraft:stone"), (10, "minecraft:stone"), (11, "minecraft:stone"), (12, "minecraft:grass_block")]);
        let r = render_chunk(&c, p, &RenderOptions { cave: true, ..Default::default() });
        assert_eq!(r.heights[0], 4);
        assert_eq!(r.surface_name(0), Some("minecraft:sand"));
        // No cave below → nothing drawn.
        let solid = chunk_with(&[(0, "minecraft:stone"), (1, "minecraft:stone")]);
        let r = render_chunk(&solid, p, &RenderOptions { cave: true, ..Default::default() });
        assert_eq!(r.pixel(0, 0)[3], 0);
    }

    #[test]
    fn empty_chunk_renders_clear() {
        let c = Chunk::default();
        let r = render_chunk(&c, Palette::embedded(), &RenderOptions::default());
        assert!(r.rgba.iter().all(|&b| b == 0));
    }
}
