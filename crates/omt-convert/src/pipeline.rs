//! The per-chunk conversion pipeline (PLAN §4 M3): `read → parse → remap →
//! serialize`, parallel over chunks with rayon — UMT's task stages
//! Load/Parse/Prepare/Convert/Clean minus Light/Render (latest-only scope).
//!
//! The pipeline is pure with respect to the target: `convert_java_chunks`
//! produces per-chunk [`ChunkConversion`] values; [`apply_conversions`]
//! writes them into a Bedrock world (delete-stale → batched put). Dry-run =
//! skip the apply step and keep the reports.

use rayon::prelude::*;

use omt_bedrock::BedrockWorld;
use omt_java::JavaWorld;
use omt_world::WorldStore;

use crate::chunk_bedrock::{ChunkConversion, ChunkWrites, java_chunk_to_bedrock};
use crate::chunk_java::{JavaChunk, parse_java_chunk};
use crate::error::{ConvertError, Result};
use crate::loss::ConversionReport;
use crate::proto::ProtoFilter;
use crate::rules::RegistryRules;

/// Pipeline configuration.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct ConvertOptions {
    pub filter: ProtoFilter,
    pub rules: RegistryRules,
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
