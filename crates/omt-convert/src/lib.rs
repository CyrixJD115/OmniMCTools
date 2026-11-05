//! Java ⇄ Bedrock conversion primitives — fixture-independent M3/M4 groundwork.
//!
//! Every behavior in this crate is grounded in the RE pass over the UMT
//! binary documented in `Source/RE/UMT_Converter_RE_Findings.md` (2026-11-03).
//! Where the findings mark a behavior as UMT-verified the doc comment cites
//! the task; where UMT's own behavior was unrecoverable (its item mappings
//! are data in a downloaded database, not code) the doc comment marks the
//! choice as an **OMT design decision** to be revisited when the M3 registry
//! generator exists. Nothing here transcribes decompiled code — formats and
//! observable behaviors only (clean-room).
//!
//! Layout:
//! - [`loss`] — the conversion report / loss ledger every converter returns
//!   (PLAN §5C: nothing is ever silently dropped).
//! - [`proto`] — the `skipProtoChunks` predicate and parse-stage routing
//!   (findings Task B).
//! - [`entities`] — entity-list key casing and Java entity-region assembly
//!   (findings Task C).
//! - [`items`] — Java item stacks (`tag` and 1.20.5+ `components` eras) ⇄
//!   Bedrock item stacks, with per-field loss reporting (findings Task A).
//! - [`rules`] — the OMT-owned conversion rule table (RON), seeded with the
//!   fixture-derived biome ids and block-palette version.
//! - [`chunk_java`] — Java chunk → conversion sidecar (modern era).
//! - [`chunk_bedrock`] — sidecar → raw Bedrock LevelDB records.
//! - [`chunk_to_java`] — decoded Bedrock chunk → Java chunk NBT root (M3b,
//!   the reverse direction).
//! - [`pipeline`] — rayon per-chunk `read → parse → remap → serialize` and
//!   the apply step (M3 converter core), both directions.

pub mod chunk_bedrock;
pub mod chunk_java;
pub mod chunk_to_java;
pub mod entities;
pub mod error;
pub mod items;
pub mod loss;
pub mod pipeline;
pub mod proto;
pub mod rules;

pub use chunk_bedrock::{ChunkConversion, ChunkWrites, java_chunk_to_bedrock};
pub use chunk_java::{JavaBiomes, JavaChunk, JavaSection, parse_java_chunk};
pub use chunk_to_java::{BedrockConversion, JavaChunkWrite, bedrock_chunk_to_java};
pub use entities::{entities_key, entity_region_root, find_entities};
pub use error::{ConvertError, Result};
pub use items::{
    EnchantIdMap, ItemConvertOpts, MixedEraPolicy, StackEra, bedrock_stack_to_java,
    java_stack_to_bedrock,
};
pub use loss::{ConversionReport, Loss, LossKind};
pub use pipeline::{
    ChunkOutcome, ConvertOptions, DimensionOutcome, JavaDimensionOutcome, apply_conversions,
    apply_java_conversions, convert_bedrock_chunks, convert_java_chunks,
};
pub use proto::{ChunkEra, ProtoFilter, SKIP_PROTO_CHUNKS_DEFAULT, is_proto_chunk};
pub use rules::{BlockRule, RegistryRules, load_rules};
