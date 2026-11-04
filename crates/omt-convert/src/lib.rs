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

pub mod entities;
pub mod error;
pub mod items;
pub mod loss;
pub mod proto;

pub use entities::{entities_key, entity_region_root, find_entities};
pub use error::{ConvertError, Result};
pub use items::{
    EnchantIdMap, ItemConvertOpts, MixedEraPolicy, StackEra, bedrock_stack_to_java,
    java_stack_to_bedrock,
};
pub use loss::{ConversionReport, Loss, LossKind};
pub use proto::{ChunkEra, ProtoFilter, SKIP_PROTO_CHUNKS_DEFAULT, is_proto_chunk};
