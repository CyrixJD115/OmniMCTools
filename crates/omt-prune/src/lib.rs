//! `omt-prune` — the World Pruner (UMT's "Pruner" app, clean-room).
//!
//! Six tools as in UMT: **Box**, **Circle**, **Polygon**, **Split** and
//! **Time** build [`Layer`]s (union of layers = selection, per-layer
//! *Inverted*), and **Optimize** compacts a Bedrock database. Pruning
//! deletes every selected chunk together with its entities and POI data.
//! Always run against a workspace copy (`omt_world::workspace`) — the
//! operation is destructive by design.

pub mod exec;
pub mod layer;

pub use exec::{ChunkInfo, PruneError, PrunePlan, PruneReport, PruneResult, PruneWorld};
pub use layer::{Layer, Selection, Shape, TimeOp};
