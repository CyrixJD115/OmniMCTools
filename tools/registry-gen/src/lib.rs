//! `registry-gen` — OMT's registry generator (PLAN §4 M3).
//!
//! Reads the pinned Mojang client.jar (blockstate **data** only — never
//! game code), Chunker's MIT registry data (attributed), the hand-authored
//! semantic maps, and emits the versioned RON rule table that
//! `omt-convert::rules` embeds, plus an assembly-stats report.
//!
//! Module map: [`model`] (registries + canonical keys) · [`java_side`] /
//! [`bedrock_side`] (inputs) · [`handmaps`] (manual semantics) ·
//! [`biomes_enchants`] (biome + enchantment tables) · [`semantic`]
//! (assembly) · [`emit`] (output).

pub mod bedrock_side;
pub mod biomes_enchants;
pub mod computed;
pub mod emit;
pub mod handmaps;
pub mod java_side;
pub mod model;
pub mod semantic;
