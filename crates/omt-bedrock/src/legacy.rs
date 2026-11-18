//! Pre-1.2.13 Bedrock chunk records, decoded into the shared model:
//!
//! - **Sub-chunk versions 0, 2–7**: `[version][4096 block ids][2048 data
//!   nibbles]` (+ light arrays we ignore), XZY order like v8.
//! - **`LegacyTerrain` (0x30, PE before 1.0)**: one 16×128×16 column —
//!   32768 ids (`x*2048 + z*128 + y`), 16384 data nibbles, sky + block
//!   light, 256 dirty-column bytes, then 256 × 4-byte biome entries
//!   (biome id, r, g, b).
//! - **`Data2D` (0x2D, 1.0–1.17)**: 512-byte heightmap + 256 biome ids;
//!   **`Data2DLegacy` (0x2E)**: heightmap + 256 × (id, r, g, b).
//!
//! Numeric ids resolve through [`omt_world::legacy::bedrock_block`].

use std::collections::BTreeMap;

use omt_world::{BedrockBiomeSlot, BedrockBiomes, Slice, legacy};

use crate::subchunk::{Subchunk, SubchunkStorage};

/// Smallest Bedrock bit width (1,2,3,4,5,6,8,16) holding `max_index`.
fn bits_for(len: usize) -> u8 {
    let need = usize::BITS - len.saturating_sub(1).leading_zeros();
    [1u8, 2, 3, 4, 5, 6, 8, 16].into_iter().find(|&b| b as u32 >= need).unwrap_or(16)
}

/// Storage from numeric `(id, data)` pairs in XZY order.
fn storage_from_ids(pairs: impl Iterator<Item = (u16, u8)>) -> SubchunkStorage {
    let (palette, indices) = legacy::palette_from_ids(pairs, legacy::bedrock_block);
    let single = palette.len() == 1;
    SubchunkStorage { bits: if single { 0 } else { bits_for(palette.len()) }, indices: (!single).then_some(indices), palette, raw_entries: None }
}

/// Decode a legacy numeric sub-chunk (versions 0 and 2–7). The result is
/// marked version 8 so a re-save writes the current palette format.
pub fn parse_numeric_subchunk(buf: &[u8], key_y: Option<i8>) -> Option<Subchunk> {
    let ids = buf.get(1..4097)?;
    let data = buf.get(4097..6145).unwrap_or(&[]);
    let storage = storage_from_ids((0..4096).map(|i| (ids[i] as u16, legacy::nibble(data, i))));
    Some(Subchunk { version: 8, y: key_y.unwrap_or(0), storages: vec![storage] })
}

/// One biome slot (repeated up the column via `NonExistent`) from 256
/// per-column ids indexed `z*16 + x`.
fn biomes_2d(ids: impl Fn(usize) -> u8) -> BedrockBiomes {
    let mut palette: Vec<i32> = Vec::new();
    let mut by_id = std::collections::HashMap::new();
    let mut col = [0u16; 256];
    for (i, c) in col.iter_mut().enumerate() {
        let id = ids(i) as i32;
        *c = *by_id.entry(id).or_insert_with(|| {
            palette.push(id);
            (palette.len() - 1) as u16
        });
    }
    let slot = if palette.len() == 1 {
        BedrockBiomeSlot::uniform(palette[0])
    } else {
        // XZY indices: x*256 + z*16 + y, the same id for all 16 Y.
        let indices = (0..4096).map(|i| col[((i >> 4) & 15) * 16 + (i >> 8)]).collect();
        BedrockBiomeSlot::Storage { bits: bits_for(palette.len()), palette, indices: Some(indices) }
    };
    let mut slots = vec![slot];
    slots.extend((1..24).map(|_| BedrockBiomeSlot::NonExistent));
    slots
}

/// Biomes of a `Data2D` (0x2D) or `Data2DLegacy` (0x2E) record, plus its
/// 256-entry heightmap.
pub fn data2d(raw: &[u8], legacy_colors: bool) -> Option<(Vec<u16>, BedrockBiomes)> {
    let heights: Vec<u16> = raw.get(..512)?.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
    let biomes = if legacy_colors {
        let b = raw.get(512..512 + 1024)?;
        biomes_2d(|i| b[i * 4])
    } else {
        let b = raw.get(512..768)?;
        biomes_2d(|i| b[i])
    };
    Some((heights, biomes))
}

/// Blocks (sections 0..8, YZX like every [`Slice`]) and biomes of a
/// pre-1.0 `LegacyTerrain` record.
pub fn legacy_terrain(raw: &[u8]) -> Option<(BTreeMap<i8, Slice>, Option<BedrockBiomes>)> {
    let ids = raw.get(..32768)?;
    let data = raw.get(32768..49152).unwrap_or(&[]);
    let mut slices = BTreeMap::new();
    for sy in 0..8usize {
        let pairs = (0..4096usize).map(|i| {
            let (y, z, x) = (i / 256, (i / 16) % 16, i % 16);
            let src = x * 2048 + z * 128 + sy * 16 + y;
            (ids[src] as u16, legacy::nibble(data, src))
        });
        let (palette, indices) = legacy::palette_from_ids(pairs, legacy::bedrock_block);
        if palette.len() > 1 {
            slices.insert(sy as i8, Slice { palette, indices });
        }
    }
    const BIOMES: usize = 32768 + 16384 * 3 + 256;
    let biomes = raw.get(BIOMES..BIOMES + 1024).map(|b| biomes_2d(|i| b[i * 4]));
    Some((slices, biomes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_subchunk_resolves_and_reencodes() {
        let mut buf = vec![0u8; 6145];
        buf[0] = 2;
        // x=1, z=2, y=3 → concrete (236) colour 4 (yellow).
        let i = (1 << 8) | (2 << 4) | 3;
        buf[1 + i] = 236;
        buf[4097 + (i >> 1)] = 4 << ((i & 1) * 4);
        buf[1] = 1; // block 0 = stone
        let sub = parse_numeric_subchunk(&buf, Some(5)).unwrap();
        assert_eq!(sub.block(1, 3, 2).unwrap().name, "minecraft:yellow_concrete");
        assert_eq!(sub.block(0, 0, 0).unwrap().name, "minecraft:stone");
        let again = Subchunk::parse(&sub.serialize(), None).unwrap();
        assert_eq!(again.block(1, 3, 2).unwrap().name, "minecraft:yellow_concrete");
    }

    #[test]
    fn data2d_biomes_per_column() {
        let mut raw = vec![0u8; 768];
        raw[512 + 3 * 16 + 5] = 21; // jungle at x=5, z=3
        raw[512] = 1;
        let (_, slots) = data2d(&raw, false).unwrap();
        let BedrockBiomeSlot::Storage { palette, indices: Some(ix), .. } = &slots[0] else { panic!() };
        assert_eq!(palette[ix[(5 << 8) | (3 << 4) | 7] as usize], 21);
        assert_eq!(palette[ix[0] as usize], 1);
        assert!(matches!(slots[1], BedrockBiomeSlot::NonExistent));
    }

    #[test]
    fn legacy_terrain_column() {
        let mut raw = vec![0u8; 32768 + 16384 * 3 + 256 + 1024];
        raw[4 * 2048 + 9 * 128 + 70] = 12; // sand at x=4, z=9, y=70
        let (slices, biomes) = legacy_terrain(&raw).unwrap();
        let s = &slices[&4];
        assert_eq!(s.palette[s.indices[6 * 256 + 9 * 16 + 4] as usize].name, "minecraft:sand");
        assert!(biomes.is_some());
    }
}
