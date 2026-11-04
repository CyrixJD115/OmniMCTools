//! The store abstraction: one API over Java files and Bedrock LevelDB.

use crate::Chunk;
use thiserror::Error;

/// Addressable world content, edition-neutral. This mirrors UMT's internal
/// node types (research report §5) minus consoles.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WorldNode {
    Chunk { dim: u8, x: i32, z: i32 },
    /// Bedrock `level.dat` / Java `level.dat` alike.
    LevelDat,
    /// Java `playerdata/<uuid>.dat`; Bedrock `~local_player` / `<uuid>`.
    PlayerFile(String),
    /// Bedrock-only: arbitrary database key.
    DbKey(Vec<u8>),
    /// Java-only: entities/r.x.z.mca region.
    EntityRegion { dim: u8, x: i32, z: i32 },
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WorldError {
    #[error("world not opened")]
    NotOpen,
    #[error("no such node: {0:?}")]
    NoSuchNode(WorldNode),
    #[error("corrupt data in {context}: {source}")]
    Corrupt {
        context: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("NBT error: {0}")]
    Nbt(#[from] omt_nbt::Error),
    #[error("world version {found} is below our read floor {floor}; open and re-save it in the current game, then retry")]
    BelowFloor { found: u64, floor: u64 },
    #[error("write refused: {0}")]
    WriteRefused(String),
    #[error("underlying store error: {0}")]
    Store(String),
}

pub type WorldResult<T> = std::result::Result<T, WorldError>;

/// Everything the higher modules (editor/converter/pruner/locator) need from
/// a world. Implementations live in `omt-java` and `omt-bedrock`.
pub trait WorldStore: Send + Sync {
    /// Decoded chunk, or `None` when the chunk is absent.
    fn read_chunk(&self, dim: u8, x: i32, z: i32) -> WorldResult<Option<Chunk>>;

    /// Write a decoded chunk back. The workspace copy-on-open model means
    /// implementations may buffer/flush lazily; call [`WorldStore::flush`].
    fn write_chunk(&self, dim: u8, x: i32, z: i32, chunk: &Chunk) -> WorldResult<()>;

    /// Delete a chunk entirely (pruner). Absent chunk is not an error.
    fn delete_chunk(&self, dim: u8, x: i32, z: i32) -> WorldResult<()>;

    /// Lazy iteration over present chunk coordinates in a dimension.
    fn iter_chunks(&self, dim: u8) -> WorldResult<Box<dyn Iterator<Item = (i32, i32)> + '_>>;

    /// Raw bytes of a node (compressed container intact where applicable).
    fn read_raw(&self, node: &WorldNode) -> WorldResult<Vec<u8>>;

    /// Persist everything buffered.
    fn flush(&mut self) -> WorldResult<()>;
}
