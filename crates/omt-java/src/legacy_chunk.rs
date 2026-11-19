//! Older Java chunk layouts, decoded into the shared [`Chunk`] model so
//! every tool (map, inspector, pruner) reads every era:
//!
//! | Era | Root | Blocks | Biomes |
//! |---|---|---|---|
//! | Beta 1.3–1.1 (McRegion `.mcr`) | `Level` | `Blocks` byte[32768] (XZY, 128 high) + `Data` | none |
//! | 1.2–1.12 (Anvil) | `Level` | `Sections[]` `Blocks`/`Add`/`Data` (YZX) | `Biomes` byte[256] |
//! | 1.13–1.15 | `Level` | `Sections[]` `Palette` + spanning `BlockStates` | int[256] / int[1024] |
//! | 1.16–1.17 | `Level` | `Sections[]` `Palette` + non-spanning `BlockStates` | int[1024] |
//!
//! The 1.18+ layout (root `sections`, `block_states`) stays in `store.rs`.

use std::collections::{BTreeMap, HashMap};

use omt_nbt::Tag;
use omt_world::{BiomeCells, Chunk, Slice, legacy};

/// First DataVersion with non-spanning packed arrays (20w17a).
const NON_SPANNING_DV: u32 = 2527;

fn bytes(t: Option<&Tag>) -> Option<&[i8]> {
    match t? {
        Tag::ByteArray(v) => Some(v),
        _ => None,
    }
}

fn u8s(v: &[i8]) -> Vec<u8> {
    v.iter().map(|&b| b as u8).collect()
}

/// Decode a `Level`-wrapped (pre-1.18) chunk into `chunk`.
pub fn decode_level(chunk: &mut Chunk, level: &Tag) {
    chunk.inhabited_time = level.get("InhabitedTime").and_then(Tag::as_i64).unwrap_or(0);
    if let Some(s) = level.get("Status").and_then(Tag::as_str) {
        chunk.status = Some(s.to_owned());
    } else if let Some(Tag::Byte(p)) = level.get("TerrainPopulated") {
        chunk.status = Some(if *p != 0 { "full".into() } else { "terrain".into() });
    }
    if let Some(sections) = level.get("Sections").and_then(Tag::as_list) {
        for section in sections {
            let Some(y) = section.get("Y").and_then(Tag::as_i64) else { continue };
            if let Some(slice) = section_slice(section, chunk.data_version) {
                chunk.slices.insert(y as i8, slice);
            }
        }
    } else if let Some(blocks) = bytes(level.get("Blocks")) {
        let data = bytes(level.get("Data")).map(u8s);
        mcregion_blocks(chunk, &u8s(blocks), data.as_deref());
    }
    biomes(chunk, level.get("Biomes"));
    if let Some(Tag::List(items)) = level.get("TileEntities") {
        chunk.block_entities = items.clone();
    }
    if let Some(Tag::List(items)) = level.get("Entities") {
        chunk.entities = items.clone();
    }
    if let Some(Tag::Compound(hm)) = level.get("Heightmaps") {
        for (name, value) in hm {
            if let Tag::LongArray(packed) = value {
                chunk.heightmaps.push((name.clone(), packed.iter().map(|&v| v as u64).collect()));
            }
        }
    }
}

/// One `Sections[]` entry (palette or numeric form).
fn section_slice(section: &Tag, data_version: u32) -> Option<Slice> {
    if let Some(pal) = section.get("Palette").and_then(Tag::as_list) {
        let palette: Vec<_> = pal.iter().map(|e| crate::store::java_palette_entry(e).unwrap_or_else(omt_world::BlockState::air)).collect();
        let indices = match section.get("BlockStates") {
            Some(Tag::LongArray(packed)) if !packed.is_empty() => {
                let spanning = data_version < NON_SPANNING_DV;
                let bits = omt_world::packing::java_legacy_block_bits(packed.len(), spanning);
                let mut ix = if spanning {
                    omt_world::packing::unpack_java_spanning(packed, 4096, bits)
                } else {
                    omt_world::packing::unpack_java_packed(packed, 4096, bits)
                };
                let max = palette.len().saturating_sub(1) as u16;
                ix.iter_mut().for_each(|i| *i = (*i).min(max));
                ix
            }
            _ => vec![0u16; 4096],
        };
        return Some(Slice { palette, indices });
    }
    let blocks = u8s(bytes(section.get("Blocks"))?);
    let add = bytes(section.get("Add")).map(u8s);
    let data = bytes(section.get("Data")).map(u8s);
    let (add, data) = (add.as_deref(), data.as_deref());
    let pairs = (0..4096).map(|i| {
        let id = blocks.get(i).copied().unwrap_or(0) as u16 | (add.map_or(0, |a| legacy::nibble(a, i)) as u16) << 8;
        (id, data.map_or(0, |d| legacy::nibble(d, i)))
    });
    let (palette, indices) = legacy::palette_from_ids(pairs, legacy::java_block);
    (palette.len() > 1).then_some(Slice { palette, indices })
}

/// Beta McRegion: one 16×128×16 block array, index `x*2048 + z*128 + y`.
fn mcregion_blocks(chunk: &mut Chunk, blocks: &[u8], data: Option<&[u8]>) {
    let height = (blocks.len() / 256).max(1);
    for sy in 0..(height / 16) as i32 {
        let pairs = (0..4096usize).map(|i| {
            let (y, z, x) = (i / 256, (i / 16) % 16, i % 16);
            let src = x * 16 * height + z * height + sy as usize * 16 + y;
            (blocks.get(src).copied().unwrap_or(0) as u16, data.map_or(0, |d| legacy::nibble(d, src)))
        });
        let (palette, indices) = legacy::palette_from_ids(pairs, legacy::java_block);
        if palette.len() > 1 {
            chunk.slices.insert(sy as i8, Slice { palette, indices });
        }
    }
}

/// Numeric biomes → per-section 4×4×4 cells (3D int[1024]: `y4<<4|z4<<2|x4`
/// over Y 0..256; 2D byte/int[256]: `z*16+x`, repeated through the column).
fn biomes(chunk: &mut Chunk, tag: Option<&Tag>) {
    let ids: Vec<u32> = match tag {
        Some(Tag::IntArray(v)) => v.iter().map(|&i| i as u32).collect(),
        Some(Tag::ByteArray(v)) => v.iter().map(|&i| i as u8 as u32).collect(),
        _ => return,
    };
    let three_d = ids.len() == 1024;
    if !three_d && ids.len() != 256 {
        return;
    }
    let sections: Vec<i8> = if three_d { (0..16).collect() } else { chunk.slices.keys().copied().collect() };
    let mut out = BTreeMap::new();
    for sy in sections {
        let mut palette: Vec<String> = Vec::new();
        let mut seen: HashMap<u32, u16> = HashMap::new();
        let mut cells = vec![0u16; 64];
        for (c, cell) in cells.iter_mut().enumerate() {
            let (y4, z4, x4) = (c >> 4, (c >> 2) & 3, c & 3);
            let id = if three_d {
                ids[((sy as usize * 4 + y4) << 4) | (z4 << 2) | x4]
            } else {
                ids[(z4 * 4 + 2) * 16 + x4 * 4 + 2]
            };
            *cell = *seen.entry(id).or_insert_with(|| {
                palette.push(legacy::biome(id).to_string());
                (palette.len() - 1) as u16
            });
        }
        out.insert(sy, BiomeCells { palette, cells });
    }
    chunk.java_biomes = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(fields: Vec<(&str, Tag)>) -> Tag {
        Tag::Compound(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn anvil_numeric_section() {
        // Stone floor at y=0, red wool (35:14) at y=1, column (3,5).
        let mut blocks = vec![0i8; 4096];
        let mut data = vec![0i8; 2048];
        blocks[..256].fill(1);
        let i = 256 + 5 * 16 + 3;
        blocks[i] = 35;
        data[i >> 1] = (14 << ((i & 1) * 4)) as i8;
        let section = level(vec![("Y", Tag::Byte(0)), ("Blocks", Tag::ByteArray(blocks)), ("Data", Tag::ByteArray(data))]);
        let lvl = level(vec![("Sections", Tag::List(vec![section])), ("Biomes", Tag::ByteArray(vec![6; 256])), ("InhabitedTime", Tag::Long(77))]);
        let mut chunk = Chunk { data_version: 1343, ..Default::default() };
        decode_level(&mut chunk, &lvl);
        assert_eq!(chunk.inhabited_time, 77);
        let s = &chunk.slices[&0];
        assert_eq!(s.palette[s.indices[0] as usize].name, "minecraft:stone");
        assert_eq!(s.palette[s.indices[i] as usize].name, "minecraft:red_wool");
        assert_eq!(chunk.java_biomes[&0].palette, vec!["minecraft:swamp".to_string()]);
    }

    #[test]
    fn flattened_spanning_section() {
        // 1.14 (DV 1976): palette of 17 entries → 5 bits, spanning.
        let palette: Vec<Tag> = (0..17).map(|i| level(vec![("Name", Tag::String(if i == 16 { "minecraft:gold_block".into() } else { format!("minecraft:stone{i}") }))])).collect();
        let mut packed = vec![0i64; 4096 * 5 / 64];
        // Block 12 (straddles longs 0/1) = palette 16.
        let (bit, v) = (12 * 5, 16u64);
        packed[bit / 64] |= (v << (bit % 64)) as i64;
        packed[bit / 64 + 1] |= (v >> (64 - bit % 64)) as i64;
        let section = level(vec![("Y", Tag::Byte(4)), ("Palette", Tag::List(palette)), ("BlockStates", Tag::LongArray(packed))]);
        let lvl = level(vec![("Sections", Tag::List(vec![section])), ("Biomes", Tag::IntArray(vec![1; 1024]))]);
        let mut chunk = Chunk { data_version: 1976, ..Default::default() };
        decode_level(&mut chunk, &lvl);
        let s = &chunk.slices[&4];
        assert_eq!(s.palette[s.indices[12] as usize].name, "minecraft:gold_block");
        assert_eq!(chunk.java_biomes[&4].palette, vec!["minecraft:plains".to_string()]);
    }

    #[test]
    fn mcregion_column() {
        let mut blocks = vec![0i8; 32768];
        // x=2, z=7, y=64 → grass.
        blocks[2 * 2048 + 7 * 128 + 64] = 2;
        let lvl = level(vec![("Blocks", Tag::ByteArray(blocks)), ("Data", Tag::ByteArray(vec![0; 16384]))]);
        let mut chunk = Chunk::default();
        decode_level(&mut chunk, &lvl);
        let s = &chunk.slices[&4];
        assert_eq!(s.palette[s.indices[7 * 16 + 2] as usize].name, "minecraft:grass_block");
        assert_eq!(chunk.slices.len(), 1);
    }
}
