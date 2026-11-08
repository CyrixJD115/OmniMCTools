//! Java chunk → Bedrock chunk-column records.
//!
//! Output is a set of raw LevelDB key/value records (built with
//! `omt_bedrock::keys` + the fixture-verified codecs) plus the loss ledger.
//! The pipeline (`crate::pipeline`) applies them to a target world.
//!
//! Format facts encoded here (see PLAN.md "Verified 1.26.52 format facts"):
//! - subchunk payload v9, block indices XZY; empty sections are omitted.
//! - data3D: 512-byte heightmap + per-slot biome records; heightmap value =
//!   topmost motion-blocking/liquid block − (−64) + 1, column order
//!   `z*16 + x` (probe: 99.6% over 10240 columns, remainder = modded decor
//!   blocks — `omt-bedrock/examples/heightprobe.rs`). Java's
//!   `MOTION_BLOCKING` heightmap uses the identical encoding, so the values
//!   copy over 1:1.
//! - chunk `Version` record = 42; `FinalizedState` = 2 (done).

use std::collections::BTreeSet;

use omt_bedrock::keys::{self, tag};
use omt_bedrock::subchunk::{Data3d, Subchunk, SubchunkStorage};
use omt_nbt::{ByteOrder, Nbt, Tag};
use omt_world::{BedrockBiomeSlot, BlockState, packing};

use crate::chunk_java::{JavaBiomes, JavaChunk, JavaSection};
use crate::loss::{ConversionReport, LossKind};
use crate::proto::{ChunkEra, ProtoFilter};
use crate::rules::RegistryRules;

/// Raw records for one converted chunk column.
#[derive(Debug, Clone)]
pub struct ChunkWrites {
    pub x: i32,
    pub z: i32,
    pub records: Vec<(Vec<u8>, Vec<u8>)>,
    pub report: ConversionReport,
}

/// Result of converting one Java chunk.
#[derive(Debug, Clone)]
pub enum ChunkConversion {
    /// Dropped at parse per `skipProtoChunks` (findings Task B).
    SkippedProto { report: ConversionReport },
    Converted(ChunkWrites),
}

/// Convert one parsed Java chunk into Bedrock records.
pub fn java_chunk_to_bedrock(
    java: &JavaChunk,
    dim: u8,
    rules: &RegistryRules,
    filter: &ProtoFilter,
) -> Result<ChunkConversion, crate::error::ConvertError> {
    let mut report = ConversionReport::new();
    if filter.should_drop(ChunkEra::Modern, java.status.as_deref()) {
        filter.record_skip(
            &mut report,
            ChunkEra::Modern,
            java.status.as_deref(),
            0,
            java.block_entities.len(),
        );
        return Ok(ChunkConversion::SkippedProto { report });
    }

    let mut records: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();

    // Subchunks: map palette, permute YZX → XZY, skip all-air sections.
    for (y, section) in &java.sections {
        if section.is_all_air() {
            continue;
        }
        let sub = subchunk_for(section, *y, rules, &mut report)?;
        records.push((
            keys::subchunk_key(java.x, java.z, dim, *y),
            sub.serialize(),
        ));
    }

    // data3D: heightmap + 3D biomes for the full column.
    let data3d = data3d_for(java, rules, &mut report);
    records.push((keys::data3d_key(java.x, java.z, dim), data3d.serialize()));

    // Chunk version + finalized state.
    records.push((keys::version_key(java.x, java.z, dim), vec![tag_version()]));
    records.push((
        keys::chunk_record(java.x, java.z, dim, tag::FINALIZED_STATE),
        2i32.to_le_bytes().to_vec(),
    ));

    // Block entities: LE NBT roots, passed through untranslated.
    if !java.block_entities.is_empty() {
        let mut blob = Vec::new();
        for be in &java.block_entities {
            blob.extend_from_slice(&le_root(be)?);
        }
        records.push((
            keys::chunk_record(java.x, java.z, dim, tag::BLOCK_ENTITIES),
            blob,
        ));
        report.record(
            LossKind::UnsupportedComponent,
            "block_entities",
            format!(
                "{} block entities passed through untranslated; field-level \
                 Java⇄Bedrock tile mapping arrives with the registry",
                java.block_entities.len()
            ),
        );
    }

    // Deferred surfaces, each recorded once per chunk.
    if java.block_ticks > 0 {
        report.record(
            LossKind::UnsupportedComponent,
            "block_ticks",
            format!("{} scheduled block ticks deferred (tick NBT mapping pending)", java.block_ticks),
        );
    }
    if java.fluid_ticks > 0 {
        report.record(
            LossKind::UnsupportedComponent,
            "fluid_ticks",
            format!("{} scheduled fluid ticks deferred (tick NBT mapping pending)", java.fluid_ticks),
        );
    }
    if java.inhabited_time > 0 {
        report.dropped("InhabitedTime", "Bedrock has no per-chunk inhabited-time home");
    }

    Ok(ChunkConversion::Converted(ChunkWrites {
        x: java.x,
        z: java.z,
        records,
        report,
    }))
}

/// Current chunk `Version` byte (fixture pin; see omt-versions registry).
fn tag_version() -> u8 {
    42
}

/// Build a v9 subchunk payload from one Java section.
fn subchunk_for(
    section: &JavaSection,
    y: i8,
    rules: &RegistryRules,
    report: &mut ConversionReport,
) -> Result<Subchunk, crate::error::ConvertError> {
    let mut mapped: Vec<BlockState> = Vec::with_capacity(section.palette.len());
    let mut index_map: Vec<u16> = Vec::with_capacity(section.palette.len());
    let mut reported: BTreeSet<String> = BTreeSet::new();
    for state in &section.palette {
        let out = rules.map_block(state, &mut reported, report);
        // Reuse the target entry when two Java states collapse together.
        let idx = match mapped.iter().position(|s| s == &out.state) {
            Some(i) => i,
            None => {
                mapped.push(out.state);
                mapped.len() - 1
            }
        };
        index_map.push(idx as u16);
    }
    let indices: Vec<u16> = section
        .indices
        .iter()
        .map(|&i| index_map.get(i as usize).copied().unwrap_or(0))
        .collect();
    let bits = bedrock_bits(mapped.len());
    let storage = SubchunkStorage {
        palette: mapped,
        raw_entries: None,
        indices: Some(packing::permute_yzx_to_xzy(&indices)),
        bits,
    };
    Ok(Subchunk { version: 9, y, storages: vec![storage] })
}

/// Bedrock bits-per-index for a palette of `len` entries (ceil log₂; 0 when
/// the palette is a single entry).
fn bedrock_bits(len: usize) -> u8 {
    if len <= 1 {
        0
    } else {
        (usize::BITS - (len - 1).leading_zeros()) as u8
    }
}

/// Build the data3D record: heightmap (Java MOTION_BLOCKING preferred —
/// identical numeric encoding to Bedrock's) + per-slot 3D biomes.
fn data3d_for(java: &JavaChunk, rules: &RegistryRules, report: &mut ConversionReport) -> Data3d {
    let heightmap = heightmap_for(java, report);
    let slots = (java.max_slot() as i32 + 5).max(16) as usize;
    let mut subchunks: Vec<BedrockBiomeSlot> = Vec::with_capacity(slots);
    let mut below_is_known = false;
    for slot in 0..slots {
        let y = slot as i8 - 4;
        let biomes = java.sections.get(&y).and_then(|s| s.biomes.as_ref());
        match biomes {
            Some(jb) => {
                subchunks.push(biome_slot(jb, rules, report));
                below_is_known = true;
            }
            None => {
                // Uncovered slot: chain NonExistent (0xFF) from the first
                // known slot up; the very bottom defaults to ocean.
                if below_is_known {
                    subchunks.push(BedrockBiomeSlot::NonExistent);
                } else {
                    subchunks.push(BedrockBiomeSlot::uniform(0));
                }
            }
        }
    }
    Data3d { heightmap, subchunks }
}

/// Heightmap: Java `MOTION_BLOCKING` values copy 1:1; fall back to
/// `WORLD_SURFACE`, then to a recompute from blocks (reported).
fn heightmap_for(java: &JavaChunk, report: &mut ConversionReport) -> Vec<u16> {
    if let Some(hm) = java.heightmaps.get("MOTION_BLOCKING") {
        return hm.clone();
    }
    if let Some(hm) = java.heightmaps.get("WORLD_SURFACE") {
        report.record(
            LossKind::ValueCoerced,
            "heightmap",
            "MOTION_BLOCKING missing; WORLD_SURFACE substituted",
        );
        return hm.clone();
    }
    report.record(
        LossKind::ValueCoerced,
        "heightmap",
        "no Java heightmaps present; recomputed as topmost non-air + 1",
    );
    let mut out = vec![0u16; 256];
    for slot_y in (-4i8..=19).rev() {
        let Some(section) = java.sections.get(&slot_y) else { continue };
        for x in 0..16usize {
            for z in 0..16usize {
                let col = z * 16 + x;
                if out[col] != 0 {
                    continue;
                }
                for iny in (0..16usize).rev() {
                    let i = (iny << 8) | (z << 4) | x; // YZX
                    let state = section
                        .palette
                        .get(section.indices.get(i).copied().unwrap_or(0) as usize);
                    if state.is_some_and(|s| s.name != BlockState::AIR) {
                        let wy = slot_y as i32 * 16 + iny as i32;
                        out[col] = (wy + 65) as u16; // height above bottom (−64) + 1
                        break;
                    }
                }
            }
        }
        if out.iter().all(|&v| v != 0) {
            break;
        }
    }
    out
}

/// One slot's biome storage: Java 4³ cells upsampled to Bedrock per-block
/// (XZY), ids mapped through the rule table (unknown → ocean + loss row).
fn biome_slot(
    jb: &JavaBiomes,
    rules: &RegistryRules,
    report: &mut ConversionReport,
) -> BedrockBiomeSlot {
    let mut reported: BTreeSet<String> = BTreeSet::new();
    let mut ids = Vec::with_capacity(jb.palette.len());
    for name in &jb.palette {
        ids.push(rules.map_biome(name, &mut reported, report));
    }
    // Id → palette position (dedup like the block storages do).
    let mut palette: Vec<i32> = Vec::new();
    let mut index_map = Vec::with_capacity(ids.len());
    for &id in &ids {
        let pos = palette.iter().position(|&p| p == id).unwrap_or_else(|| {
            palette.push(id);
            palette.len() - 1
        });
        index_map.push(pos as u16);
    }
    if palette.len() <= 1 {
        return BedrockBiomeSlot::uniform(palette.first().copied().unwrap_or(0));
    }
    let mut indices = vec![0u16; 4096];
    for x in 0..16usize {
        for z in 0..16usize {
            for y in 0..16usize {
                let cell = ((y >> 2) << 4) | ((z >> 2) << 2) | (x >> 2);
                let java_idx = jb.cells.get(cell).copied().unwrap_or(0) as usize;
                let target = index_map.get(java_idx).copied().unwrap_or(0);
                indices[omt_bedrock::subchunk::block_index_xzy(x, y, z)] = target;
            }
        }
    }
    let bits = bedrock_bits(palette.len());
    BedrockBiomeSlot::Storage { palette, indices: Some(indices), bits }
}

/// Encode one rootless Java compound as a Bedrock little-endian NBT root
/// (compound tag, empty root name) — the 0x31 record's entry format.
pub fn le_root(tag: &Tag) -> Result<Vec<u8>, crate::error::ConvertError> {
    if !matches!(tag, Tag::Compound(_)) {
        return Err(crate::error::ConvertError::BadField {
            field: "block_entities",
            detail: "block entity is not a compound".into(),
        });
    }
    omt_nbt::write(&Nbt { name: String::new(), tag: tag.clone() }, ByteOrder::Little)
        .map_err(|e| crate::error::ConvertError::BadField {
            field: "block_entities",
            detail: format!("NBT re-encode failed: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk_java::parse_java_chunk;
    use omt_nbt::snbt::from_snbt;

    fn sample_nbt() -> Nbt {
        // Two sections; top has stone at (x=1,y=2,z=3) in YZX order, plus a
        // stateful block to exercise the drop path.
        let palette = Tag::List(vec![
            Tag::Compound(vec![("Name".into(), Tag::String("minecraft:air".into()))]),
            Tag::Compound(vec![("Name".into(), Tag::String("minecraft:stone".into()))]),
            Tag::Compound(vec![
                ("Name".into(), Tag::String("minecraft:oak_stairs".into())),
                (
                    "Properties".into(),
                    Tag::Compound(vec![("facing".into(), Tag::String("east".into()))]),
                ),
            ]),
        ]);
        let mut data = vec![0i64; 256]; // 4 bits × 4096, all zero → air
        let mut set = |x: usize, y: usize, z: usize, v: i64| {
            let i = (y << 8) | (z << 4) | x;
            data[i / 16] |= v << ((i % 16) * 4);
        };
        set(1, 2, 3, 1); // stone
        set(4, 5, 6, 2); // stairs (properties dropped)
        Nbt {
            name: String::new(),
            tag: Tag::Compound(vec![
                ("DataVersion".into(), Tag::Int(3953)),
                ("xPos".into(), Tag::Int(0)),
                ("zPos".into(), Tag::Int(0)),
                ("Status".into(), Tag::String("minecraft:full".into())),
                (
                    "Heightmaps".into(),
                    Tag::Compound(vec![(
                        "MOTION_BLOCKING".into(),
                        Tag::LongArray(vec![130; 37]),
                    )]),
                ),
                (
                    "sections".into(),
                    Tag::List(vec![Tag::Compound(vec![
                        ("Y".into(), Tag::Byte(0)),
                        (
                            "block_states".into(),
                            Tag::Compound(vec![
                                ("palette".into(), palette),
                                ("data".into(), Tag::LongArray(data)),
                            ]),
                        ),
                        (
                            "biomes".into(),
                            Tag::Compound(vec![(
                                "palette".into(),
                                Tag::List(vec![Tag::String("minecraft:plains".into())]),
                            )]),
                        ),
                    ])]),
                ),
            ]),
        }
    }

    #[test]
    fn converts_blocks_biomes_and_heightmap() {
        let java = parse_java_chunk(&sample_nbt()).unwrap();
        let out = java_chunk_to_bedrock(&java, 0, &RegistryRules::default(), &ProtoFilter::default())
            .unwrap();
        let ChunkConversion::Converted(writes) = out else { panic!("expected conversion") };

        // Loss ledger: stairs properties dropped.
        assert_eq!(writes.report.count_with_kind(LossKind::UnmappedId), 1);
        assert!(!writes.report.is_clean());

        // Subchunk: stone survives the permutation at the XZY position.
        let sc_raw = &writes
            .records
            .iter()
            .find(|(k, _)| k.len() == 10 && k[8] == 0x2f)
            .map(|(_, v)| v.clone())
            .expect("subchunk record");
        let sub = Subchunk::parse(sc_raw, Some(0)).unwrap();
        assert_eq!(sub.storages.len(), 1);
        assert_eq!(sub.block(1, 2, 3).unwrap().name, "minecraft:stone");
        let stairs = sub.block(4, 5, 6).unwrap();
        assert_eq!(stairs.name, "minecraft:oak_stairs");
        assert!(stairs.properties.is_empty(), "unmapped properties dropped");

        // data3D: uniform plains (1) slot + 1:1 heightmap.
        let d3_raw = &writes
            .records
            .iter()
            .find(|(k, _)| k.len() == 9 && k[8] == 0x2b)
            .map(|(_, v)| v.clone())
            .expect("data3d record");
        let d3 = Data3d::parse(d3_raw).unwrap();
        assert_eq!(d3.heightmap[0], 130);
        // Uncovered bottom slots default to ocean; the section's slot (4,
        // world Y 0) carries the mapped plains id. biome_at's y is in
        // slot space (0 = bottom slot), matching its fixture tests.
        assert_eq!(d3.biome_at(0, 0, 0), Some(0));
        assert_eq!(d3.biome_at(0, 64, 0), Some(1));

        // Version + finalized records present.
        assert!(writes.records.iter().any(|(k, v)| k.len() == 9 && k[8] == 0x2c && v.as_slice() == [42]));
        assert!(writes.records.iter().any(|(k, v)| k.len() == 9 && k[8] == 0x36 && v.as_slice() == 2i32.to_le_bytes()));
    }

    #[test]
    fn proto_chunk_is_skipped_with_report() {
        let mut nbt = sample_nbt();
        if let Tag::Compound(fields) = &mut nbt.tag {
            for (k, v) in fields.iter_mut() {
                if k == "Status" {
                    *v = Tag::String("minecraft:empty".into());
                }
            }
        }
        let java = parse_java_chunk(&nbt).unwrap();
        let out =
            java_chunk_to_bedrock(&java, 0, &RegistryRules::default(), &ProtoFilter::default())
                .unwrap();
        assert!(matches!(out, ChunkConversion::SkippedProto { .. }));
    }

    #[test]
    fn biome_upsampling_maps_cells_to_blocks() {
        // Cells: y cell 0 = plains(1), y cell 1 = desert(2): block y<4 → 1.
        let jb = JavaBiomes {
            palette: vec!["minecraft:plains".into(), "minecraft:desert".into()],
            cells: {
                let mut c = vec![0u16; 64];
                for slot in c.iter_mut().skip(16) {
                    *slot = 1;
                }
                c
            },
        };
        let slot = biome_slot(&jb, &RegistryRules::default(), &mut ConversionReport::new());
        let BedrockBiomeSlot::Storage { palette, indices: Some(indices), .. } = slot else {
            panic!("expected storage");
        };
        assert_eq!(palette, vec![1, 2]);
        assert_eq!(indices[block_index_xzy(0, 0, 0)], 0);
        assert_eq!(indices[block_index_xzy(0, 4, 0)], 1);
        assert_eq!(indices[block_index_xzy(15, 15, 15)], 1);
    }

    #[test]
    fn le_root_round_trips() {
        let tag = from_snbt(r#"{id: "minecraft:chest", x: 1, Items: []}"#).unwrap();
        let bytes = le_root(&tag).unwrap();
        let (back, consumed) = omt_nbt::read_prefix(&bytes, ByteOrder::Little).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(back.tag.get("id").and_then(Tag::as_str), Some("minecraft:chest"));
    }

    use omt_bedrock::subchunk::block_index_xzy;
}
