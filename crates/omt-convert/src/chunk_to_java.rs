//! Bedrock chunk column → Java chunk NBT root (M3b, the reverse direction).
//!
//! Input is the already-decoded universal [`Chunk`] from
//! `BedrockWorld::read_chunk_full` — block indices are YZX-normalized at the
//! store boundary, so no permutation happens here (the Java side is YZX
//! native). Output is an NBT root ready for `RegionWriter::put` plus the
//! loss ledger.
//!
//! Java write shape (minecraft.wiki *Chunk format*, fetched 2026-11-04;
//! cross-checked against what `omt-java::store::nbt_to_chunk` and
//! `omt-convert::chunk_java::parse_java_chunk` read — where any source
//! disagreed, the reader we already have wins):
//! - root: `DataVersion` Int · `xPos`/`yPos`/`zPos` Int (`yPos` = lowest
//!   section, −4) · `Status` String (`minecraft:full`, namespaced) ·
//!   `InhabitedTime` Long · `sections` · `Heightmaps` · `block_entities` ·
//!   empty `block_ticks`/`fluid_ticks` lists · `PostProcessing` = 24 empty
//!   lists (one per section, bottom-up).
//! - `sections[]`: `Y` Byte · `block_states.palette` [{`Name`, `Properties`}]
//!   with `data` **omitted when the palette has ≤1 entry** (4-bit minimum
//!   width otherwise) · `biomes.palette` [String] + 64-cell `data` (omitted
//!   at ≤1 biome).
//! - `Heightmaps`: 37 longs × 9 bits, 7 values per long, non-spanning
//!   (`omt_world::packing::pack_java_heightmap`). Column order is z-major
//!   on the Bedrock side (probe fact); Java's order is assumed identical
//!   (PLAN §7 open row, pending Java fixture) so values copy 1:1.
//! - Deliberately omitted: `isLightOn` (we do not compute light), `LastUpdate`,
//!   `structures` — optional fields our reader ignores and vanilla fills
//!   with defaults. Documented OMT-design choices, listed in PLAN §7.
//!
//! Entity handling lives with the pipeline (region-file assembly); this
//! module carries each chunk's actors through in [`JavaChunkWrite::entities`]
//! with the honest per-chunk loss rows.

use std::collections::BTreeMap;

use omt_nbt::{ByteOrder, Nbt, Tag};
use omt_world::{BedrockBiomeSlot, Chunk, packing};

use crate::loss::{ConversionReport, LossKind};
use crate::proto::{ChunkEra, ProtoFilter};
use crate::rules::RegistryRules;

/// The Java `Status` written for converted terrain (namespaced — the
/// modern-era proto filter keeps only `full`/`minecraft:full`, findings
/// Task B).
pub const JAVA_FULL_STATUS: &str = "minecraft:full";

/// Lowest Java section Y for current worlds (−64..319); also the root
/// `yPos` value.
pub const JAVA_MIN_SECTION: i8 = -4;

/// One converted chunk column, ready to write into a Java world.
#[derive(Debug, Clone)]
pub struct JavaChunkWrite {
    pub x: i32,
    pub z: i32,
    /// Big-endian NBT root bytes (the `RegionWriter::put` payload).
    pub chunk_nbt: Vec<u8>,
    /// Actor compounds carried through to the Java entity region for this
    /// chunk (opaque passthrough after the `identifier`→`id` rename).
    pub entities: Vec<Tag>,
    pub report: ConversionReport,
}

/// Result of converting one Bedrock chunk column.
#[derive(Debug, Clone)]
pub enum BedrockConversion {
    /// Dropped at parse per `skipProtoChunks` (findings Task B). Bedrock
    /// reads synthesize `full`, so this only fires for callers that hand
    /// us non-full statuses.
    SkippedProto { report: ConversionReport },
    Converted(JavaChunkWrite),
}

/// Convert one decoded Bedrock chunk column into a Java chunk NBT root.
///
/// `data_version` is the Java `DataVersion` to stamp (the workspace
/// registry pin, `omt_versions::registry().java_data_version` — provisional
/// Java pin, cited not hardcoded).
pub fn bedrock_chunk_to_java(
    chunk: &Chunk,
    rules: &RegistryRules,
    filter: &ProtoFilter,
    data_version: u32,
) -> Result<BedrockConversion, crate::error::ConvertError> {
    let mut report = ConversionReport::new();
    if filter.should_drop(ChunkEra::Modern, chunk.status.as_deref()) {
        filter.record_skip(
            &mut report,
            ChunkEra::Modern,
            chunk.status.as_deref(),
            chunk.entities.len(),
            chunk.block_entities.len(),
        );
        return Ok(BedrockConversion::SkippedProto { report });
    }

    let mut sections: Vec<Tag> = Vec::new();
    // The Bedrock→Java state inverse, derived once per chunk from the one
    // rule table (see `rules::ReverseBlockRules`).
    let reverse = rules.reverse();
    for (slot_y, slice) in &chunk.slices {
        if is_all_air(slice) {
            continue; // empty sections are omitted entirely
        }
        let section = section_nbt(*slot_y, slice, chunk, rules, &reverse, &mut report)?;
        sections.push(section);
    }

    let heightmap = heightmap_for_java(chunk, &mut report);
    let mut root = vec![
        ("DataVersion".to_string(), Tag::Int(data_version as i32)),
        ("xPos".to_string(), Tag::Int(chunk.x)),
        ("yPos".to_string(), Tag::Int(JAVA_MIN_SECTION as i32)),
        ("zPos".to_string(), Tag::Int(chunk.z)),
        ("Status".to_string(), Tag::String(JAVA_FULL_STATUS.into())),
        ("InhabitedTime".to_string(), Tag::Long(chunk.inhabited_time)),
        (
            "Heightmaps".to_string(),
            Tag::Compound(vec![(
                "MOTION_BLOCKING".to_string(),
                Tag::LongArray(packing::pack_java_heightmap(&heightmap)),
            )]),
        ),
        ("sections".to_string(), Tag::List(sections)),
        // Empty by construction: tick NBT mapping is deferred (same ledger
        // row policy as the forward direction).
        ("block_ticks".to_string(), Tag::List(Vec::new())),
        ("fluid_ticks".to_string(), Tag::List(Vec::new())),
        (
            "PostProcessing".to_string(),
            Tag::List((0..24).map(|_| Tag::List(Vec::new())).collect()),
        ),
    ];
    if !chunk.block_entities.is_empty() {
        root.push(("block_entities".to_string(), Tag::List(chunk.block_entities.clone())));
        report.record(
            LossKind::UnsupportedComponent,
            "block_entities",
            format!(
                "{} block entities passed through untranslated; field-level \
                 Bedrock→Java tile mapping arrives with the registry",
                chunk.block_entities.len()
            ),
        );
    }

    let entities = java_entities(chunk, &mut report);
    let chunk_nbt = omt_nbt::write(
        &Nbt { name: String::new(), tag: Tag::Compound(root) },
        ByteOrder::Big,
    )
    .map_err(|e| crate::error::ConvertError::BadField {
        field: "chunk root",
        detail: format!("NBT encode failed: {e}"),
    })?;

    Ok(BedrockConversion::Converted(JavaChunkWrite {
        x: chunk.x,
        z: chunk.z,
        chunk_nbt,
        entities,
        report,
    }))
}

/// True when every block of the slice is air (the empty-section rule).
fn is_all_air(slice: &omt_world::Slice) -> bool {
    slice
        .indices
        .iter()
        .all(|&i| slice.palette.get(i as usize).is_none_or(|b| b.name == omt_world::BlockState::AIR))
}

/// Build one section compound: block palette + data, biomes downsampled
/// from the Bedrock per-block record to Java 4³ cells.
fn section_nbt(
    slot_y: i8,
    slice: &omt_world::Slice,
    chunk: &Chunk,
    rules: &RegistryRules,
    reverse: &crate::rules::ReverseBlockRules,
    report: &mut ConversionReport,
) -> Result<Tag, crate::error::ConvertError> {
    // Palette: Bedrock → Java through the derived inverse table (exact
    // state inverse, then name inverse, then honest passthrough). Collapse
    // duplicates exactly like the forward path does.
    let mut palette: Vec<omt_world::BlockState> = Vec::with_capacity(slice.palette.len());
    let mut index_map = Vec::with_capacity(slice.palette.len());
    let mut reported = std::collections::BTreeSet::new();
    for state in &slice.palette {
        let out = reverse.map_block(state, &mut reported, report);
        let idx = match palette.iter().position(|s| s == &out.state) {
            Some(i) => i,
            None => {
                palette.push(out.state);
                palette.len() - 1
            }
        };
        index_map.push(idx as u16);
    }
    let indices: Vec<u16> = slice
        .indices
        .iter()
        .map(|&i| index_map.get(i as usize).copied().unwrap_or(0))
        .collect();

    let mut fields = vec![
        ("Y".to_string(), Tag::Byte(slot_y)),
        (
            "block_states".to_string(),
            Tag::Compound(block_states_nbt(&palette, &indices)),
        ),
    ];
    if let Some(biomes) = section_biomes(chunk, slot_y, rules, report) {
        fields.push(("biomes".to_string(), biomes));
    } else {
        report.record(
            LossKind::FieldDropped,
            format!("sections[{slot_y}].biomes"),
            "no Bedrock biome data covers this slot; section written without \
             biomes (the target game re-derives them)",
        );
    }
    Ok(Tag::Compound(fields))
}

/// `block_states` compound: palette entries (Properties only when present)
/// + `data` long array (omitted at palette ≤ 1, `java_block_bits` width).
fn block_states_nbt(
    palette: &[omt_world::BlockState],
    indices: &[u16],
) -> Vec<(String, Tag)> {
    let entries: Vec<Tag> = palette
        .iter()
        .map(|state| {
            let mut fields = vec![("Name".to_string(), Tag::String(state.name.clone()))];
            if !state.properties.is_empty() {
                fields.push((
                    "Properties".to_string(),
                    Tag::Compound(
                        state
                            .properties
                            .iter()
                            .map(|(k, v)| (k.clone(), Tag::String(v.clone())))
                            .collect(),
                    ),
                ));
            }
            Tag::Compound(fields)
        })
        .collect();
    let mut fields = vec![("palette".to_string(), Tag::List(entries))];
    let data = packing::pack_java_indices(indices, palette.len());
    if !data.is_empty() {
        fields.push(("data".to_string(), Tag::LongArray(data)));
    }
    fields
}

/// One slot's `biomes` compound: Bedrock per-block (4096, XZY) downsampled
/// to Java 4³ cells (64) by deterministic majority vote. Bedrock variance
/// inside one cell is unrepresentable in Java — one aggregate `ValueCoerced`
/// row per chunk counts the affected cells. `NonExistent` slots resolve to
/// the nearest storage below.
fn section_biomes(
    chunk: &Chunk,
    slot_y: i8,
    rules: &RegistryRules,
    report: &mut ConversionReport,
) -> Option<Tag> {
    let slots = chunk.bedrock_biomes.as_ref()?;
    let slot_idx = (slot_y as i32 - JAVA_MIN_SECTION as i32) as usize;
    let mut reported_ids = std::collections::BTreeSet::new();

    let (palette_ids, cell_ids): (Vec<i32>, Vec<u16>) = match resolve_slot(slots, slot_idx) {
        ResolvedSlot::Missing => return None,
        ResolvedSlot::Uniform(id) => (vec![id], vec![0u16; 64]),
        ResolvedSlot::PerBlock { palette, indices } => {
            let mut cells = vec![0u16; 64];
            let mut cell_palette: Vec<i32> = Vec::new();
            let mut coerced_cells = 0usize;
            for cy in 0..4usize {
                for cz in 0..4usize {
                    for cx in 0..4usize {
                        let (id, mixed) = vote_cell(palette, indices, cx, cy, cz);
                        let pos = match cell_palette.iter().position(|&p| p == id) {
                            Some(p) => p,
                            None => {
                                cell_palette.push(id);
                                cell_palette.len() - 1
                            }
                        };
                        cells[(cy << 4) | (cz << 2) | cx] = pos as u16;
                        coerced_cells += usize::from(mixed);
                    }
                }
            }
            if coerced_cells > 0 {
                report.record(
                    LossKind::ValueCoerced,
                    format!("sections[{slot_y}].biomes"),
                    format!(
                        "{coerced_cells} of 64 biome cells had more than one Bedrock \
                         biome inside the 4×4×4 cell; downsampled by majority vote \
                         (lowest palette index wins ties)"
                    ),
                );
            }
            (cell_palette, cells)
        }
    };

    let biome_names = rules.biome_names();
    let names: Vec<String> = palette_ids
        .iter()
        .map(|&id| rules.map_biome_to_java(&biome_names, id, &mut reported_ids, report))
        .collect();
    let name_positions: BTreeMap<&str, u16> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i as u16))
        .collect();
    // Dedup: cells reference palette positions; collapse equal names.
    let java_palette: Vec<String> = {
        let mut seen = Vec::new();
        for n in &names {
            if !seen.contains(n) {
                seen.push(n.clone());
            }
        }
        seen
    };
    let remap: Vec<u16> = names.iter().map(|n| name_positions[n.as_str()]).collect();
    let cells: Vec<u16> = cell_ids.iter().map(|&c| remap[c as usize]).collect();

    let mut fields = vec![(
        "palette".to_string(),
        Tag::List(java_palette.iter().map(|n| Tag::String(n.clone())).collect()),
    )];
    let data = packing::pack_java_biome_cells(&cells, java_palette.len());
    if !data.is_empty() {
        fields.push(("data".to_string(), Tag::LongArray(data)));
    }
    Some(Tag::Compound(fields))
}

/// The biome source for one slot after the `NonExistent` (0xFF) chain is
/// resolved toward the nearest storage below (same semantics as
/// `Data3d::biome_at`).
enum ResolvedSlot<'a> {
    /// No biome data anywhere at or below this slot.
    Missing,
    /// A uniform storage: one biome id covers the whole slot.
    Uniform(i32),
    /// A per-block storage: palette + 4096 XZY indices.
    PerBlock { palette: &'a [i32], indices: &'a [u16] },
}

fn resolve_slot<'a>(slots: &'a [BedrockBiomeSlot], slot_idx: usize) -> ResolvedSlot<'a> {
    if slots.is_empty() {
        return ResolvedSlot::Missing;
    }
    let mut s = slot_idx.min(slots.len() - 1);
    loop {
        match &slots[s] {
            BedrockBiomeSlot::NonExistent => {
                if s == 0 {
                    return ResolvedSlot::Missing;
                }
                s -= 1;
            }
            BedrockBiomeSlot::Storage { palette, indices, .. } => match indices.as_deref() {
                Some(ix) => return ResolvedSlot::PerBlock { palette, indices: ix },
                None => return ResolvedSlot::Uniform(palette.first().copied().unwrap_or(0)),
            },
        }
    }
}

/// Majority vote over the 4×4×4 blocks of one cell (Bedrock XZY indices).
/// Returns the winning biome id and whether the cell was mixed. Ties go to
/// the lowest palette position (first appearance in the Bedrock palette).
fn vote_cell(
    palette: &[i32],
    indices: &[u16],
    cx: usize,
    cy: usize,
    cz: usize,
) -> (i32, bool) {
    let mut counts = std::collections::BTreeMap::new();
    for dy in 0..4usize {
        for dz in 0..4usize {
            for dx in 0..4usize {
                let x = cx * 4 + dx;
                let y = cy * 4 + dy;
                let z = cz * 4 + dz;
                let pos = indices
                    .get(omt_bedrock::subchunk::block_index_xzy(x, y, z))
                    .copied()
                    .unwrap_or(0) as usize;
                *counts.entry(pos).or_insert(0u32) += 1;
            }
        }
    }
    let mixed = counts.len() > 1;
    let best = counts
        .into_iter()
        .max_by_key(|&(pos, count)| (count, std::cmp::Reverse(pos)))
        .map(|(pos, _)| pos)
        .unwrap_or(0);
    (palette.get(best).copied().unwrap_or(0), mixed)
}

/// Java `MOTION_BLOCKING` values: Bedrock's data3D 2D map copies 1:1
/// (identical numeric encoding, z-major on both sides per the probe fact —
/// Java-side column order is the standing PLAN §7 assumption). Missing
/// data3D recomputes topmost-non-air + 1, reported.
fn heightmap_for_java(chunk: &Chunk, report: &mut ConversionReport) -> Vec<u16> {
    if let Some((_, hm)) = chunk.heightmaps.first() {
        let mut out = vec![0u16; 256];
        for (i, &v) in hm.iter().take(256).enumerate() {
            out[i] = v as u16;
        }
        return out;
    }
    report.record(
        LossKind::ValueCoerced,
        "heightmap",
        "no Bedrock data3D heightmap present; recomputed as topmost non-air + 1",
    );
    let mut out = vec![0u16; 256];
    for slot_y in (JAVA_MIN_SECTION..=19).rev() {
        let Some(slice) = chunk.slices.get(&slot_y) else { continue };
        for x in 0..16usize {
            for z in 0..16usize {
                let col = z * 16 + x; // z-major, x fastest (probe fact)
                if out[col] != 0 {
                    continue;
                }
                for iny in (0..16usize).rev() {
                    let i = (iny << 8) | (z << 4) | x; // YZX
                    let state = slice
                        .palette
                        .get(slice.indices.get(i).copied().unwrap_or(0) as usize);
                    if state.is_some_and(|s| s.name != omt_world::BlockState::AIR) {
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

/// Actor passthrough for the Java entity region. Grounded empirically
/// (probe `omt-bedrock/examples/actorprobe.rs` over the 1.26.52 fixture,
/// 2026-11-04): every actor carries `identifier` (namespaced string) +
/// `UniqueID` (Long) and **no** `UUID` IntArray. Java entities carry `id`,
/// so the rename is applied here; a missing UUID is a per-chunk loss row —
/// never invented.
fn java_entities(chunk: &Chunk, report: &mut ConversionReport) -> Vec<Tag> {
    if chunk.entities.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(chunk.entities.len());
    let mut renamed = 0usize;
    let mut no_uuid = 0usize;
    for actor in &chunk.entities {
        let Tag::Compound(fields) = actor else { continue };
        let mut converted: Vec<(String, Tag)> = fields
            .iter()
            .map(|(k, v)| {
                if k == "identifier" {
                    renamed += 1;
                    ("id".to_string(), v.clone())
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect();
        if !converted.iter().any(|(k, _)| k == "UUID") {
            no_uuid += 1;
        }
        out.push(Tag::Compound(std::mem::take(&mut converted)));
    }
    if renamed > 0 {
        report.record(
            LossKind::ValueCoerced,
            "entities",
            format!("{renamed} actors' `identifier` renamed to Java's `id`"),
        );
    }
    if no_uuid > 0 {
        report.record(
            LossKind::FieldDropped,
            "entities",
            format!(
                "{no_uuid} actors carry no Java UUID (Bedrock's UniqueID is a numeric \
                 id, not a UUID); written without one — not invented"
            ),
        );
    }
    report.record(
        LossKind::UnsupportedComponent,
        "entities",
        format!(
            "{} actors passed through untranslated; Bedrock-only fields and \
             per-entity mapping arrive with the registry (M4)",
            chunk.entities.len()
        ),
    );
    out
}

