//! `omt-world` — the edition-neutral world domain shared by every other crate.
//!
//! Types model UMT's decoded chunk structs (addendum §1) simplified to the
//! latest-only scope: both editions share the 1.18+ coordinate space
//! (Y −64..319), so no height shifting exists anywhere in the tool.

pub mod detect;
pub mod packing;
pub mod path;
pub mod store;

pub use detect::{detect, Detected, WorldSummary, WorldSource};
pub use packing::{
    permute_xzy_to_yzx, permute_yzx_to_xzy, unpack_java_biome_cells, unpack_java_heightmap,
    unpack_java_indices, unpack_java_packed,
};
pub use path::VPath;
pub use store::{WorldError, WorldNode, WorldResult, WorldStore};

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edition {
    Java,
    Bedrock,
}

impl Edition {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Java => "Java Edition",
            Self::Bedrock => "Bedrock Edition",
        }
    }
}

/// The three dimensions, keyed the Bedrock way (0/1/2). Java's directory
/// layout (DIM-1/DIM1) maps onto this in `omt-java`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
}

impl Dimension {
    pub const ALL: [Dimension; 3] = [Self::Overworld, Self::Nether, Self::End];

    pub const fn id(self) -> u8 {
        match self {
            Self::Overworld => 0,
            Self::Nether => 1,
            Self::End => 2,
        }
    }

    pub const fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::Overworld),
            1 => Some(Self::Nether),
            2 => Some(Self::End),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Overworld => "Overworld",
            Self::Nether => "Nether",
            Self::End => "The End",
        }
    }
}

/// A block state in canonical (edition-agnostic) form: namespaced name plus
/// ordered properties. Java uses this shape natively; Bedrock blocks are
/// normalized to it after resolving through the per-world palette.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlockState {
    pub name: String,
    pub properties: Vec<(String, String)>,
}

impl BlockState {
    pub const AIR: &'static str = "minecraft:air";

    pub fn air() -> Self {
        Self { name: Self::AIR.into(), properties: Vec::new() }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.properties.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// One 16×16 block layer of a chunk column (either a Java section or one
/// Bedrock subchunk Y-slice of 16). `indices` has 4096 entries in **YZX**
/// order (`y << 8 | z << 4 | x`) — the Java order and the universal model's
/// order. Bedrock's on-disk XZY order is normalized at the store boundary
/// (see `packing`); it used to be copied in verbatim, which silently broke
/// `Chunk::block_at` for Bedrock reads (fixed 2026-11-04).
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    pub palette: Vec<BlockState>,
    pub indices: Vec<u16>,
}

/// Bedrock-only per-subchunk biome storage (from the `0x2B` data3D record).
/// Slot `i` covers Y `i*16 - 64 .. i*16 - 49`. `NonExistent` (0xFF marker)
/// means "same as the slot below". Indices are per-block (4096) in XZY order.
#[derive(Debug, Clone, PartialEq)]
pub enum BedrockBiomeSlot {
    NonExistent,
    Storage {
        palette: Vec<i32>,
        /// 4096 palette indices; `None` = single-biome palette (bits == 0).
        indices: Option<Vec<u16>>,
        /// On-disk bit width, kept for byte-faithful re-encode.
        bits: u8,
    },
}

impl BedrockBiomeSlot {
    pub fn uniform(biome: i32) -> Self {
        Self::Storage { palette: vec![biome], indices: None, bits: 0 }
    }
}

/// One column's biome slots, bottom to top.
pub type BedrockBiomes = Vec<BedrockBiomeSlot>;

/// A decoded chunk column. `slices` is keyed by Y index of the 16-block layer:
/// −4..=19 for a full-height world (−64..319).
#[derive(Debug, Clone, Default)]
pub struct Chunk {
    pub x: i32,
    pub z: i32,
    pub dim: u8,
    /// Java `DataVersion`; Bedrock chunks carry a per-subchunk version instead.
    pub data_version: u32,
    /// Java `Status` (e.g. "minecraft:full"); Bedrock synthesizes "full".
    pub status: Option<String>,
    /// Pruner Time-layer input.
    pub inhabited_time: i64,
    pub slices: BTreeMap<i8, Slice>,
    /// Java: packed heightmaps by name; Bedrock: 2D u16 heightmap equivalent.
    pub heightmaps: Vec<(String, Vec<u64>)>,
    /// Both current editions store biomes per 4×4×4 cell in-section.
    pub biomes_3d: bool,
    /// Bedrock data3D biome record (see [`BedrockBiomes`]).
    pub bedrock_biomes: Option<BedrockBiomes>,
    pub block_entities: Vec<omt_nbt::Tag>,
    pub entities: Vec<omt_nbt::Tag>,
    pub block_ticks: Vec<omt_nbt::Tag>,
    pub fluid_ticks: Vec<omt_nbt::Tag>,
}

impl Chunk {
    pub fn dimension(&self) -> Option<Dimension> {
        Dimension::from_id(self.dim)
    }

    /// Block at world-space Y (any value in −64..=319 for current worlds).
    pub fn block_at(&self, local_x: usize, y: i32, local_z: usize) -> Option<&BlockState> {
        let slice_y = y.div_euclid(16) as i8;
        let slice = self.slices.get(&slice_y)?;
        let in_y = y.rem_euclid(16) as usize;
        // YZX: index = y*256 + z*16 + x
        let idx = slice.indices.get(in_y * 256 + local_z * 16 + local_x)?;
        slice.palette.get(*idx as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yzx_ordering_and_negative_slices() {
        let mut chunk = Chunk { x: 0, z: 0, dim: 0, ..Default::default() };
        let stone = BlockState { name: "minecraft:stone".into(), properties: vec![] };
        chunk.slices.insert(
            -4,
            Slice { palette: vec![BlockState::air(), stone.clone()], indices: vec![0; 4096] },
        );
        assert_eq!(chunk.block_at(0, -64, 0), Some(&BlockState::air()));
        let s = chunk.slices.get(&-4).unwrap();
        let mut indices = [0u16; 4096];
        indices[15 * 256 + 15 * 16 + 15] = 1; // top-most block of the slice
        let s2 = Slice { palette: s.palette.clone(), indices: indices.to_vec() };
        chunk.slices.insert(-4, s2);
        assert_eq!(chunk.block_at(15, -64 + 15, 15), Some(&stone));
    }
}
