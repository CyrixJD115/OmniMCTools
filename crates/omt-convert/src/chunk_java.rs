//! Java chunk sidecar: a conversion-oriented view of one chunk's NBT root.
//!
//! Modern era only (1.18+ flat root with `sections`) — the workspace read
//! floor is DataVersion 2865 and UMT's own converter predates most legacy
//! layouts' relevance; a `Level`-rooted chunk is a typed error, not a
//! guess. Blocks are decoded with the non-spanning 1.16+ packing
//! (`omt_world::packing`); indices stay in Java's YZX order.

use std::collections::BTreeMap;

use omt_nbt::{Nbt, Tag};
use omt_world::{BlockState, packing};

use crate::error::{ConvertError, Result};

/// One section's biome palette (Java 1.18+ `biomes` compound).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaBiomes {
    /// Java biome ids (`minecraft:plains`).
    pub palette: Vec<String>,
    /// 64 cell indices, cell order `(y << 4) | (z << 2) | x`.
    pub cells: Vec<u16>,
}

/// One 16³ section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaSection {
    pub y: i8,
    /// Block palette (`Name` + optional `Properties`).
    pub palette: Vec<BlockState>,
    /// 4096 palette indices, YZX order (Java native).
    pub indices: Vec<u16>,
    pub biomes: Option<JavaBiomes>,
}

impl JavaSection {
    /// True when the section contains nothing but air (or is empty).
    pub fn is_all_air(&self) -> bool {
        self.palette.iter().all(|b| b.name == BlockState::AIR)
            || self.indices.iter().all(|&i| {
                self.palette.get(i as usize).is_none_or(|b| b.name == BlockState::AIR)
            })
    }
}

/// Conversion-oriented Java chunk.
#[derive(Debug, Clone, Default)]
pub struct JavaChunk {
    pub x: i32,
    pub z: i32,
    pub data_version: u32,
    /// Java `Status` (`minecraft:full` for converted terrain).
    pub status: Option<String>,
    pub inhabited_time: i64,
    /// Present sections by Y slot (−4..=19).
    pub sections: BTreeMap<i8, JavaSection>,
    /// Unpacked heightmaps by name (`MOTION_BLOCKING`, `WORLD_SURFACE`, …);
    /// 256 u16 values, column order x + z*16, values "height above world
    /// bottom (−64) + 1" — numerically identical to Bedrock's data3D map.
    pub heightmaps: BTreeMap<String, Vec<u16>>,
    /// Block entities (rootless compounds, as stored in the list).
    pub block_entities: Vec<Tag>,
    pub block_ticks: usize,
    pub fluid_ticks: usize,
}

impl JavaChunk {
    /// Highest populated Y slot, for data3D sizing (bottom slot is −4).
    pub fn max_slot(&self) -> i8 {
        self.sections.keys().copied().max().unwrap_or(-4)
    }
}

/// Parse a modern-era Java chunk root.
pub fn parse_java_chunk(nbt: &Nbt) -> Result<JavaChunk> {
    let root = &nbt.tag;
    if root.get("Level").is_some() && root.get("sections").is_none() {
        return Err(ConvertError::LegacyChunk(
            "chunk root carries a `Level` compound (pre-1.18 layout); the latest-only \
             read floor does not convert legacy chunks",
        ));
    }
    let mut chunk = JavaChunk {
        x: root.get("xPos").and_then(Tag::as_i64).unwrap_or(0) as i32,
        z: root.get("zPos").and_then(Tag::as_i64).unwrap_or(0) as i32,
        data_version: root.get("DataVersion").and_then(Tag::as_i64).unwrap_or(0) as u32,
        status: root.get("Status").and_then(Tag::as_str).map(str::to_owned),
        inhabited_time: root.get("InhabitedTime").and_then(Tag::as_i64).unwrap_or(0),
        ..Default::default()
    };
    if let Some(Tag::List(sections)) = root.get("sections") {
        for section in sections {
            let Some(y) = section.get("Y").and_then(Tag::as_i64) else { continue };
            let Some(bs) = section.get("block_states") else { continue };
            let mut palette = Vec::new();
            if let Some(Tag::List(entries)) = bs.get("palette") {
                for entry in entries {
                    let name = entry
                        .get("Name")
                        .and_then(Tag::as_str)
                        .ok_or(ConvertError::BadField {
                            field: "palette.Name",
                            detail: "block palette entry without a Name string".into(),
                        })?
                        .to_string();
                    let mut properties = Vec::new();
                    if let Some(Tag::Compound(props)) = entry.get("Properties") {
                        for (k, v) in props {
                            if let Tag::String(s) = v {
                                properties.push((k.clone(), s.clone()));
                            }
                        }
                    }
                    palette.push(BlockState { name, properties });
                }
            }
            let indices = match bs.get("data") {
                Some(Tag::LongArray(packed)) => {
                    packing::unpack_java_indices(packed, palette.len())
                }
                _ => vec![0u16; 4096], // palette ≤ 1 ⇒ no data field
            };
            let biomes = section.get("biomes").and_then(|bi| {
                let bpalette: Vec<String> = match bi.get("palette") {
                    Some(Tag::List(entries)) => {
                        entries.iter().filter_map(|e| e.as_str().map(str::to_owned)).collect()
                    }
                    _ => Vec::new(),
                };
                let cells = match bi.get("data") {
                    Some(Tag::LongArray(packed)) => {
                        packing::unpack_java_biome_cells(packed, bpalette.len())
                    }
                    _ => vec![0u16; 64],
                };
                (!bpalette.is_empty()).then_some(JavaBiomes { palette: bpalette, cells })
            });
            chunk.sections.insert(
                y as i8,
                JavaSection { y: y as i8, palette, indices, biomes },
            );
        }
    }
    if let Some(Tag::Compound(hms)) = root.get("Heightmaps") {
        for (name, value) in hms {
            if let Tag::LongArray(packed) = value {
                chunk.heightmaps.insert(name.clone(), packing::unpack_java_heightmap(packed));
            }
        }
    }
    if let Some(Tag::List(bes)) = root.get("block_entities") {
        chunk.block_entities = bes.clone();
    }
    chunk.block_ticks = tick_count(root.get("block_ticks"));
    chunk.fluid_ticks = tick_count(root.get("fluid_ticks"));
    Ok(chunk)
}

fn tick_count(tag: Option<&Tag>) -> usize {
    match tag {
        Some(Tag::List(items)) => items.len(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omt_nbt::ByteOrder;

    /// Build a minimal modern Java chunk: two sections, second stateful.
    fn sample_chunk() -> Nbt {
        let section = |y: i8, blocks: &[&str], data: &[i64], biome: &str| {
            let palette: Vec<Tag> = blocks
                .iter()
                .map(|b| {
                    Tag::Compound(vec![("Name".to_string(), Tag::String(b.to_string()))])
                })
                .collect();
            let entries: Vec<Tag> = [biome]
                .iter()
                .map(|b| Tag::String(b.to_string()))
                .collect();
            let mut section = vec![
                ("Y".to_string(), Tag::Byte(y)),
                (
                    "block_states".to_string(),
                    Tag::Compound(vec![
                        ("palette".to_string(), Tag::List(palette)),
                        ("data".to_string(), Tag::LongArray(data.to_vec())),
                    ]),
                ),
                (
                    "biomes".to_string(),
                    Tag::Compound(vec![
                        ("palette".to_string(), Tag::List(entries)),
                    ]),
                ),
            ];
            if biome.is_empty() {
                section.pop();
            }
            Tag::Compound(section)
        };
        // Palette [air, stone]: index 1 everywhere in the top slot.
        let data = vec![0x1111_1111_1111_1111i64; 256];
        Nbt {
            name: String::new(),
            tag: Tag::Compound(vec![
                ("DataVersion".to_string(), Tag::Int(3953)),
                ("xPos".to_string(), Tag::Int(7)),
                ("zPos".to_string(), Tag::Int(-9)),
                ("Status".to_string(), Tag::String("minecraft:full".into())),
                (
                    "Heightmaps".to_string(),
                    Tag::Compound(vec![(
                        "MOTION_BLOCKING".to_string(),
                        Tag::LongArray(vec![100; 37]),
                    )]),
                ),
                (
                    "block_entities".to_string(),
                    Tag::List(vec![Tag::Compound(vec![
                        ("id".to_string(), Tag::String("minecraft:chest".into())),
                        ("x".to_string(), Tag::Int(112)),
                        ("y".to_string(), Tag::Int(0)),
                        ("z".to_string(), Tag::Int(-145)),
                    ])]),
                ),
                (
                    "sections".to_string(),
                    Tag::List(vec![section(-4, &["minecraft:air"], &[], "minecraft:plains"), section(-3, &["minecraft:air", "minecraft:stone"], &data, "minecraft:plains")]),
                ),
            ]),
        }
    }

    #[test]
    fn parses_sections_heightmaps_and_entities() {
        let nbt = sample_chunk();
        let chunk = parse_java_chunk(&nbt).unwrap();
        assert_eq!((chunk.x, chunk.z), (7, -9));
        assert_eq!(chunk.data_version, 3953);
        assert_eq!(chunk.sections.len(), 2);
        let top = chunk.sections.get(&-3).unwrap();
        assert_eq!(top.palette.len(), 2);
        assert!(!top.is_all_air());
        assert!(chunk.sections.get(&-4).unwrap().is_all_air());
        // Non-spanning decode: 0x1111… = every 4-bit nibble is 1 ⇒ index 1.
        assert_eq!(top.indices[0], 1);
        assert_eq!(top.indices[4095], 1);
        assert_eq!(top.biomes.as_ref().unwrap().palette, vec!["minecraft:plains"]);
        let hm = chunk.heightmaps.get("MOTION_BLOCKING").unwrap();
        assert_eq!(hm.len(), 256);
        assert_eq!(hm[0], 100);
        assert_eq!(chunk.block_entities.len(), 1);
        assert_eq!(chunk.max_slot(), -3);
    }

    #[test]
    fn legacy_chunk_root_is_a_typed_error() {
        let legacy = Nbt {
            name: String::new(),
            tag: Tag::Compound(vec![(
                "Level".to_string(),
                Tag::Compound(vec![("Status".to_string(), Tag::String("full".into()))]),
            )]),
        };
        assert!(matches!(parse_java_chunk(&legacy), Err(ConvertError::LegacyChunk(_))));
        // Sanity: the bytes must also parse as NBT for other callers.
        assert!(omt_nbt::write(&legacy, ByteOrder::Big).is_ok());
    }
}
