//! `skipProtoChunks` — proto-chunk detection and parse-stage routing.
//!
//! UMT-verified behavior (findings Task B, `ChunkTaskParse::run`
//! `0x45AC30`): the filter runs at the *parse* stage, defaults to **on**,
//! and drops the chunk wholesale — its entities and block entities are
//! never converted. The predicate is era-aware and intentionally
//! asymmetric (faithful to the binary):
//!
//! - **Legacy era** (pre-1.18 layout, status read via `Level.Status`):
//!   proto ⇔ status ∈ {`base`, `empty`, `structure_starts`}. Other partial
//!   statuses (`noise`, `surface`, `light`, …) are *kept* and converted.
//! - **Modern era** (1.18+ / Bedrock universal layout, status at root):
//!   proto ⇔ status ≠ `full`, after stripping one optional `minecraft:`
//!   prefix. Only `full` (or `minecraft:full`) survives.
//! - Missing status / missing `Level` ⇒ kept (not proto).

use crate::loss::ConversionReport;

/// Chunk layout era — mirrors UMT's universal-chunk era byte
/// (`chunk+0x14` in the findings; 1 = legacy, 2 = modern).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChunkEra {
    /// Pre-1.18 layout: fields under `Level`, `Status` inside it.
    Legacy,
    /// 1.18+ / Bedrock-universal layout: flat root, `Status` at root.
    Modern,
}

/// UMT writes `true` for `converter/general/options/skipProtoChunks` when
/// the key is absent from settings (findings Task B: defaults writer
/// `FUN_0046c1a0`, `QVariant(bool)` with `1`).
pub const SKIP_PROTO_CHUNKS_DEFAULT: bool = true;

/// The statuses that count as "proto" in the legacy era, exact set from
/// the binary (`base` @ 0x5f81c4, `empty` @ 0x5f81cc,
/// `structure_starts` @ 0x5f81d4).
pub const LEGACY_PROTO_STATUSES: [&str; 3] = ["base", "empty", "structure_starts"];

/// The only status that survives the modern-era filter.
pub const MODERN_FULL_STATUS: &str = "full";

/// Strip the optional `minecraft:` prefix UMT strips before comparing in
/// the **modern-era** branch only (`QString::startsWith("minecraft:")` +
/// `remove(0, 10)`, findings Task B). The legacy-era comparisons in the
/// binary are raw (no prefix handling).
fn strip_namespace(status: &str) -> &str {
    status.strip_prefix("minecraft:").unwrap_or(status)
}

/// Would UMT skip this chunk as "proto"? Pure predicate; the caller owns
/// what "skip" means (see [`ProtoFilter::should_drop`]).
///
/// `status` is the chunk's `Status` string if present. It must already be
/// read from the era-correct location: `Level.Status` for
/// [`ChunkEra::Legacy`], root `Status` for [`ChunkEra::Modern`]
/// (see [`crate::entities::find_entities`] for the same era-split reads;
/// M3 wires this to `omt_world::Chunk::status`).
pub fn is_proto_chunk(era: ChunkEra, status: Option<&str>) -> bool {
    match (era, status) {
        (ChunkEra::Legacy, Some(status)) => {
            LEGACY_PROTO_STATUSES.contains(&status)
        }
        // Missing status is treated as non-proto in both eras (binary:
        // a failed `Status` lookup skips the check entirely).
        (ChunkEra::Modern, Some(status)) => strip_namespace(status) != MODERN_FULL_STATUS,
        (_, None) => false,
    }
}

/// Parse-stage routing for one chunk (findings Task B "OMT implementation
/// note": options struct with `skip_proto_chunks: bool = true`, skipped
/// chunks and their entity/block-entity counts surface in the report).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtoFilter {
    /// Default [`SKIP_PROTO_CHUNKS_DEFAULT`] (on).
    pub skip_proto_chunks: bool,
}

impl Default for ProtoFilter {
    fn default() -> Self {
        Self {
            skip_proto_chunks: SKIP_PROTO_CHUNKS_DEFAULT,
        }
    }
}

impl ProtoFilter {
    pub fn new(skip: bool) -> Self {
        Self { skip_proto_chunks: skip }
    }

    /// Should this chunk be dropped at parse? When true, the chunk *and
    /// everything nested in it* (entities, block entities) is not
    /// converted — count them with [`ProtoFilter::record_skip`].
    pub fn should_drop(&self, era: ChunkEra, status: Option<&str>) -> bool {
        self.skip_proto_chunks && is_proto_chunk(era, status)
    }

    /// Record one dropped chunk into the world report, phrased per the
    /// findings ("N proto chunks skipped (≈M entities dropped)").
    pub fn record_skip(
        &self,
        report: &mut ConversionReport,
        era: ChunkEra,
        status: Option<&str>,
        entities: usize,
        block_entities: usize,
    ) {
        if !self.should_drop(era, status) {
            return;
        }
        report.record(
            crate::loss::LossKind::FieldDropped,
            "chunk",
            format!(
                "proto chunk skipped at parse (status {:?}, era {:?}) with {entities} entities \
                 and {block_entities} block entities",
                status.unwrap_or("<none>"),
                era,
            ),
        );
    }
}

/// One-line summary for the conversion log: "N proto chunks skipped
/// (≈M entities, K block entities dropped)".
pub fn format_skip_summary(chunks: usize, entities: usize, block_entities: usize) -> String {
    format!("{chunks} proto chunks skipped (≈{entities} entities, {block_entities} block entities dropped)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_proto_statuses_match_binary_exactly() {
        assert!(is_proto_chunk(ChunkEra::Legacy, Some("base")));
        assert!(is_proto_chunk(ChunkEra::Legacy, Some("empty")));
        assert!(is_proto_chunk(ChunkEra::Legacy, Some("structure_starts")));
        // Partial-but-terrain statuses are KEPT in the legacy era — the
        // documented asymmetry (findings Task B edge cases).
        assert!(!is_proto_chunk(ChunkEra::Legacy, Some("noise")));
        assert!(!is_proto_chunk(ChunkEra::Legacy, Some("surface")));
        assert!(!is_proto_chunk(ChunkEra::Legacy, Some("light")));
        assert!(!is_proto_chunk(ChunkEra::Legacy, Some("full")));
        assert!(!is_proto_chunk(ChunkEra::Legacy, None));
    }

    #[test]
    fn modern_keeps_only_full() {
        assert!(!is_proto_chunk(ChunkEra::Modern, Some("full")));
        assert!(!is_proto_chunk(ChunkEra::Modern, Some("minecraft:full")));
        for status in ["base", "empty", "structure_starts", "noise", "light", "spawn"] {
            assert!(
                is_proto_chunk(ChunkEra::Modern, Some(status)),
                "modern era must drop {status}"
            );
        }
        // Namespaced partial statuses drop too, after prefix stripping.
        assert!(is_proto_chunk(ChunkEra::Modern, Some("minecraft:features")));
        assert!(!is_proto_chunk(ChunkEra::Modern, None));
    }

    #[test]
    fn filter_defaults_on_and_records_counts() {
        let filter = ProtoFilter::default();
        assert!(filter.skip_proto_chunks);
        assert!(filter.should_drop(ChunkEra::Modern, Some("noise")));
        assert!(!filter.should_drop(ChunkEra::Modern, Some("full")));
        let off = ProtoFilter::new(false);
        assert!(!off.should_drop(ChunkEra::Modern, Some("empty")));

        let mut report = ConversionReport::new();
        filter.record_skip(&mut report, ChunkEra::Legacy, Some("empty"), 7, 2);
        assert_eq!(report.losses.len(), 1);
        assert!(report.losses[0].detail.contains("7 entities"));
        assert!(report.losses[0].detail.contains("2 block entities"));
        // Non-proto chunk records nothing.
        filter.record_skip(&mut report, ChunkEra::Modern, Some("full"), 9, 9);
        assert_eq!(report.losses.len(), 1);
    }

    #[test]
    fn summary_phrase_matches_findings_style() {
        let s = format_skip_summary(4, 11, 3);
        assert_eq!(s, "4 proto chunks skipped (≈11 entities, 3 block entities dropped)");
    }
}
