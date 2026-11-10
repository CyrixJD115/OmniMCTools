//! The per-chunk conversion pipeline (PLAN §4 M3): `read → parse → remap →
//! serialize`, parallel over chunks with rayon — UMT's task stages
//! Load/Parse/Prepare/Convert/Clean minus Light/Render (latest-only scope).
//!
//! The pipelines are pure with respect to the target:
//! - [`convert_java_chunks`] produces per-chunk Bedrock record writes;
//!   [`apply_conversions`] applies them to a Bedrock world.
//! - [`convert_bedrock_chunks`] (M3b, the reverse direction) produces
//!   per-chunk Java NBT payloads + carried entity lists;
//!   [`apply_java_conversions`] writes them into a Java world folder
//!   (region files + entity regions, merged with existing content).
//!
//! Dry-run = skip the apply step and keep the reports.

use rayon::prelude::*;
use std::collections::BTreeMap;
use std::path::Path;

use omt_bedrock::BedrockWorld;
use omt_java::{JavaWorld, RegionWriter, region::RegionFile};
use omt_nbt::{ByteOrder, Container, Nbt, Tag};
use omt_world::WorldStore;

use crate::chunk_bedrock::{ChunkConversion, ChunkWrites, java_chunk_to_bedrock};
use crate::chunk_java::{JavaChunk, parse_java_chunk};
use crate::chunk_to_java::{BedrockConversion, JavaChunkWrite, bedrock_chunk_to_java};
use crate::entities::entity_region_root;
use crate::error::{ConvertError, Result};
use crate::loss::ConversionReport;
use crate::proto::ProtoFilter;
use crate::rules::RegistryRules;

/// Pipeline configuration. The default rule table is the embedded
/// generated registry (`tools/registry-gen` output) — build a custom
/// `ConvertOptions` to override it.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub filter: ProtoFilter,
    pub rules: RegistryRules,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self { filter: ProtoFilter::default(), rules: RegistryRules::embedded().clone() }
    }
}


/// One chunk's pipeline outcome.
#[derive(Debug, Clone)]
pub enum ChunkOutcome {
    Converted(Box<ChunkWrites>),
    SkippedProto { report: ConversionReport },
    Failed { error: String },
}

/// Aggregate result of converting every Java chunk of one dimension.
#[derive(Debug, Default)]
pub struct DimensionOutcome {
    pub converted: Vec<(i32, i32)>,
    /// Raw records for every converted chunk, ready to apply.
    pub writes: Vec<ChunkWrites>,
    pub skipped_proto: usize,
    pub failed: Vec<(i32, i32, String)>,
    /// Per-chunk loss ledgers, in conversion order.
    pub reports: Vec<ConversionReport>,
}

impl DimensionOutcome {
    /// Merged loss ledger (one row per loss across all chunks).
    pub fn merged_report(&self) -> ConversionReport {
        let mut merged = ConversionReport::new();
        for r in &self.reports {
            merged.extend(r);
        }
        merged
    }
}

/// Convert every Java chunk in `dim` (read → parse → remap → serialize).
/// No writes happen here; see [`apply_conversions`].
pub fn convert_java_chunks(
    java: &JavaWorld,
    dim: u8,
    opts: &ConvertOptions,
) -> Result<DimensionOutcome> {
    let coords = java
        .list_chunks(dim)
        .map_err(|e| ConvertError::Store(format!("listing Java chunks: {e}")))?;

    let outcomes: Vec<(i32, i32, ChunkOutcome)> = coords
        .par_iter()
        .map(|&(x, z)| {
            let outcome = convert_one(java, dim, x, z, opts);
            (x, z, outcome)
        })
        .collect();

    let mut result = DimensionOutcome::default();
    for (x, z, outcome) in outcomes {
        match outcome {
            ChunkOutcome::Converted(writes) => {
                result.reports.push(writes.report.clone());
                result.converted.push((writes.x, writes.z));
                result.writes.push(*writes);
            }
            ChunkOutcome::SkippedProto { report } => {
                result.reports.push(report);
                result.skipped_proto += 1;
            }
            ChunkOutcome::Failed { error } => result.failed.push((x, z, error)),
        }
    }
    result.converted.sort_unstable();
    Ok(result)
}

/// Pipeline stages for one chunk. Parse errors are per-chunk failures, not
/// pipeline aborts — one corrupt chunk must not stop a world conversion.
fn convert_one(
    java: &JavaWorld,
    dim: u8,
    x: i32,
    z: i32,
    opts: &ConvertOptions,
) -> ChunkOutcome {
    let nbt = match java.read_chunk_nbt(dim, x, z) {
        Ok(Some(nbt)) => nbt,
        Ok(None) => {
            return ChunkOutcome::Failed { error: "chunk vanished between list and read".into() };
        }
        Err(e) => return ChunkOutcome::Failed { error: format!("read: {e}") },
    };
    let parsed: Result<JavaChunk> = parse_java_chunk(&nbt)
        .map_err(|e| ConvertError::Store(format!("parse ({x},{z}): {e}")));
    let parsed = match parsed {
        Ok(p) => p,
        Err(e) => return ChunkOutcome::Failed { error: e.to_string() },
    };
    match java_chunk_to_bedrock(&parsed, dim, &opts.rules, &opts.filter) {
        Ok(ChunkConversion::Converted(writes)) => ChunkOutcome::Converted(Box::new(writes)),
        Ok(ChunkConversion::SkippedProto { report }) => ChunkOutcome::SkippedProto { report },
        Err(e) => ChunkOutcome::Failed { error: e.to_string() },
    }
}

/// Apply converted records to a writable Bedrock world: delete the chunk
/// column's existing records first (stale subchunks must not survive a
/// re-conversion), then batched-put the new ones. Consumes the writes out
/// of `outcome`; a dry run simply never calls this.
pub fn apply_conversions(
    bedrock: &BedrockWorld,
    dim: u8,
    outcome: &mut DimensionOutcome,
) -> Result<usize> {
    let writes = std::mem::take(&mut outcome.writes);
    let mut applied = 0usize;
    for w in &writes {
        bedrock
            .delete_chunk(dim, w.x, w.z)
            .map_err(|e| ConvertError::Store(format!("clearing ({},{}): {e}", w.x, w.z)))?;
        bedrock
            .raw_put_many(&w.records)
            .map_err(|e| ConvertError::Store(format!("writing ({},{}): {e}", w.x, w.z)))?;
        applied += 1;
    }
    Ok(applied)
}

// ---------------------------------------------------------------------------
// Reverse direction (M3b): Bedrock → Java
// ---------------------------------------------------------------------------

/// One chunk's reverse-direction outcome.
#[derive(Debug, Clone)]
pub enum BedrockOutcome {
    Converted(Box<JavaChunkWrite>),
    SkippedProto { report: ConversionReport },
    Failed { error: String },
}

/// Aggregate result of converting every Bedrock chunk of one dimension
/// toward Java.
#[derive(Debug, Default)]
pub struct JavaDimensionOutcome {
    pub converted: Vec<(i32, i32)>,
    /// Java NBT payloads + carried entity lists, ready for
    /// [`apply_java_conversions`].
    pub writes: Vec<JavaChunkWrite>,
    pub skipped_proto: usize,
    pub failed: Vec<(i32, i32, String)>,
    pub reports: Vec<ConversionReport>,
    /// The Java `DataVersion` stamped on chunks and entity-region roots
    /// (workspace registry pin).
    pub data_version: u32,
}

impl JavaDimensionOutcome {
    /// Merged loss ledger (one row per loss across all chunks).
    pub fn merged_report(&self) -> ConversionReport {
        let mut merged = ConversionReport::new();
        for r in &self.reports {
            merged.extend(r);
        }
        merged
    }
}

/// Convert every Bedrock chunk in `dim` (read → parse → remap → serialize)
/// toward Java. No writes happen here; see [`apply_java_conversions`].
///
/// Reads run **serially**: every `BedrockWorld` get goes through one
/// LevelDB-mutex guard, so parallel readers only add convoy overhead
/// (measured: parallel reads were several times slower than serial on the
/// 1.26.52 fixture). The CPU-bound conversion stage parallelizes in
/// bounded batches to keep peak memory flat.
pub fn convert_bedrock_chunks(
    bedrock: &BedrockWorld,
    dim: u8,
    opts: &ConvertOptions,
) -> Result<JavaDimensionOutcome> {
    let coords = bedrock
        .list_chunks(dim)
        .map_err(|e| ConvertError::Store(format!("listing Bedrock chunks: {e}")))?;
    let data_version = omt_versions::registry().java_data_version;

    const READ_BATCH: usize = 64;
    let mut result = JavaDimensionOutcome {
        data_version,
        ..Default::default()
    };
    for batch in coords.chunks(READ_BATCH) {
        let read: Vec<(i32, i32, BedrockOutcome)> = batch
            .iter()
            .map(|&(x, z)| {
                let outcome = match bedrock.read_chunk_full(dim, x, z) {
                    Ok(Some(chunk)) => match bedrock_chunk_to_java(
                        &chunk,
                        &opts.rules,
                        &opts.filter,
                        data_version,
                    ) {
                        Ok(BedrockConversion::Converted(writes)) => {
                            BedrockOutcome::Converted(Box::new(writes))
                        }
                        Ok(BedrockConversion::SkippedProto { report }) => {
                            BedrockOutcome::SkippedProto { report }
                        }
                        Err(e) => BedrockOutcome::Failed { error: e.to_string() },
                    },
                    Ok(None) => BedrockOutcome::Failed {
                        error: "chunk vanished between list and read".into(),
                    },
                    Err(e) => BedrockOutcome::Failed { error: format!("read: {e}") },
                };
                (x, z, outcome)
            })
            .collect();
        for (x, z, outcome) in read {
            match outcome {
                BedrockOutcome::Converted(writes) => {
                    result.reports.push(writes.report.clone());
                    result.converted.push((writes.x, writes.z));
                    result.writes.push(*writes);
                }
                BedrockOutcome::SkippedProto { report } => {
                    result.reports.push(report);
                    result.skipped_proto += 1;
                }
                BedrockOutcome::Failed { error } => result.failed.push((x, z, error)),
            }
        }
    }
    result.converted.sort_unstable();
    Ok(result)
}

/// Apply converted chunks to a Java world folder: scaffold a minimal
/// `level.dat` when the folder has none, then write/merge region files
/// (`region/`, per [`omt_java::store::dimension_dir`]) and entity regions
/// (`entities/`, per [`omt_java::store::entity_dimension_dir`]). Existing chunks
/// of rewritten regions are carried over untouched; existing entity-region
/// payloads keep their raw bytes. `level_name` is only used for the
/// scaffold (full `level.dat` field mapping is M4). Consumes the writes;
/// a dry run never calls this.
pub fn apply_java_conversions(
    java_root: &Path,
    dim: u8,
    outcome: &mut JavaDimensionOutcome,
    level_name: Option<&str>,
) -> Result<usize> {
    if !java_root.join("level.dat").is_file() {
        scaffold_java_world(java_root, level_name, outcome.data_version)?;
    }
    let writes = std::mem::take(&mut outcome.writes);

    // Group by region so each region file is rewritten once.
    let mut by_region: BTreeMap<(i32, i32), Vec<&JavaChunkWrite>> = BTreeMap::new();
    for w in &writes {
        let key = (w.x.div_euclid(32), w.z.div_euclid(32));
        by_region.entry(key).or_default().push(w);
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    let converted_keys: std::collections::BTreeSet<(i32, i32)> =
        writes.iter().map(|w| (w.x, w.z)).collect();

    for (&(rx, rz), chunks) in &by_region {
        // Terrain region: converted payloads + carried-over existing chunks.
        let mut writer = RegionWriter::new();
        merge_existing(
            &omt_java::store::dimension_dir(java_root, dim).join(format!("r.{rx}.{rz}.mca")),
            rx,
            rz,
            &converted_keys,
            &mut writer,
            outcome,
        )?;
        for w in chunks {
            writer.put(w.x, w.z, w.chunk_nbt.clone(), timestamp);
        }
        let dir = omt_java::store::dimension_dir(java_root, dim);
        std::fs::create_dir_all(&dir)
            .map_err(|e| ConvertError::Store(format!("creating {}: {e}", dir.display())))?;
        std::fs::write(dir.join(format!("r.{rx}.{rz}.mca")), writer.finish())
            .map_err(|e| ConvertError::Store(format!("writing region r.{rx}.{rz}: {e}")))?;

        // Entity region: one root per converted chunk (findings Task C
        // shape; carried opaquely from `read_chunk_full`'s digp walk).
        let mut entities_writer = RegionWriter::new();
        merge_existing(
            &omt_java::store::entity_dimension_dir(java_root, dim).join(format!("r.{rx}.{rz}.mca")),
            rx,
            rz,
            &converted_keys,
            &mut entities_writer,
            outcome,
        )?;
        for w in chunks {
            let root = entity_region_root(w.x, w.z, outcome.data_version as i32, w.entities.clone());
            let raw = omt_nbt::write(&Nbt { name: String::new(), tag: root }, ByteOrder::Big)
                .map_err(|e| ConvertError::Store(format!("entity root NBT: {e}")))?;
            entities_writer.put(w.x, w.z, raw, timestamp);
        }
        let edir = omt_java::store::entity_dimension_dir(java_root, dim);
        std::fs::create_dir_all(&edir)
            .map_err(|e| ConvertError::Store(format!("creating {}: {e}", edir.display())))?;
        std::fs::write(edir.join(format!("r.{rx}.{rz}.mca")), entities_writer.finish())
            .map_err(|e| ConvertError::Store(format!("writing entities r.{rx}.{rz}: {e}")))?;
    }
    Ok(writes.len())
}

/// Carry every existing chunk of an on-disk region file (skipping chunks
/// the conversion replaces) into `writer`. An unreadable existing file is
/// reported and replaced — never silently.
fn merge_existing(
    path: &Path,
    rx: i32,
    rz: i32,
    converted: &std::collections::BTreeSet<(i32, i32)>,
    writer: &mut RegionWriter,
    outcome: &mut JavaDimensionOutcome,
) -> Result<()> {
    let Ok(bytes) = std::fs::read(path) else {
        return Ok(()); // no existing region
    };
    let region = match RegionFile::from_bytes(path, &bytes) {
        Ok(r) => r,
        Err(e) => {
            let mut r = ConversionReport::new();
            r.record(
                crate::loss::LossKind::FieldDropped,
                format!("existing {}", path.display()),
                format!("unreadable, replaced: {e}"),
            );
            outcome.reports.push(r);
            return Ok(());
        }
    };
    for (x, z) in region.chunk_coords(rx, rz) {
        if converted.contains(&(x, z)) {
            continue;
        }
        let ts = region.timestamp(x, z).unwrap_or(0);
        let raw = region.read_raw(&bytes, x, z).map_err(|e| {
            ConvertError::Store(format!("carrying over ({x},{z}) from {}: {e}", path.display()))
        })?;
        writer.put(x, z, raw, ts);
    }
    Ok(())
}

/// Minimal Java world scaffold (OMT design — full `level.dat` field
/// mapping is M4): a gzip `Data` compound with `DataVersion` (registry
/// pin) and `LevelName`. The target game fills every other field with its
/// defaults.
fn scaffold_java_world(root: &Path, level_name: Option<&str>, data_version: u32) -> Result<()> {
    let nbt = Nbt {
        name: String::new(),
        tag: Tag::Compound(vec![(
            "Data".to_string(),
            Tag::Compound(vec![
                ("DataVersion".to_string(), Tag::Int(data_version as i32)),
                (
                    "LevelName".to_string(),
                    Tag::String(level_name.unwrap_or("Converted World").to_string()),
                ),
            ]),
        )]),
    };
    std::fs::create_dir_all(root)
        .map_err(|e| ConvertError::Store(format!("creating {}: {e}", root.display())))?;
    let bytes = Container::Gzip
        .encode(&omt_nbt::write(&nbt, ByteOrder::Big).map_err(|e| ConvertError::Store(format!("scaffold NBT: {e}")))?);
    std::fs::write(root.join("level.dat"), bytes)
        .map_err(|e| ConvertError::Store(format!("writing {}: {e}", root.join("level.dat").display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merged_report_accumulates() {
        let mut a = ConversionReport::new();
        a.dropped("x", "one");
        let mut b = ConversionReport::new();
        b.dropped("y", "two");
        let out = DimensionOutcome {
            converted: vec![(0, 0), (1, 1)],
            writes: vec![],
            skipped_proto: 1,
            failed: vec![],
            reports: vec![a, b],
        };
        assert_eq!(out.merged_report().losses.len(), 2);
        assert_eq!(out.converted.len(), 2);
    }
}
