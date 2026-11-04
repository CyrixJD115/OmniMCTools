//! LevelDB key codec for Bedrock worlds.
//!
//! Layout **verified against the 1.26.52 `world_ref` fixtures** (see
//! `examples/probe.rs`; byte evidence in the M1 notes):
//! - chunk base: `[i32 LE x][i32 LE z]`, plus a dimension byte `1`/`2` for
//!   nether/end (overworld has none), plus a one-byte record tag.
//! - `0x2F` + `i8` Y: subchunk; `0x2B` data3D (heightmap + 3D biomes);
//!   `0x2C` chunk version (u8); `0x31` block entities (LE NBT);
//!   `0x33`/`0x3A` block/fluid ticks; `0x36` finalized state;
//!   `0x77` structure references.
//! - `digp` + chunk base: actor-digest list; `actorprefix` + 8-byte id:
//!   actor NBT; `~local_player`, `BiomeData`, `Overworld`, …

/// Record tag bytes (suffix on a chunk base key).
pub mod tag {
    /// Heightmap (256×u16) + 3D-biome palette + 1024 cell indices.
    pub const DATA_3D: u8 = 0x2b;
    /// Current chunk version (single u8, 42 in the 1.26.52 fixtures).
    pub const VERSION: u8 = 0x2c;
    /// Legacy pre-1.18 2D data.
    pub const DATA_2D_LEGACY: u8 = 0x2d;
    pub const SUBCHUNK_PREFIX: u8 = 0x2f;
    /// Block entities per chunk, little-endian NBT.
    pub const BLOCK_ENTITIES: u8 = 0x31;
    /// Block ticks ("currentTick"/"tickList" NBT).
    pub const BLOCK_TICKS: u8 = 0x33;
    pub const EDGES: u8 = 0x35;
    /// Finalized state (i32 LE; 2 = done).
    pub const FINALIZED_STATE: u8 = 0x36;
    /// Fluid ticks.
    pub const FLUID_TICKS: u8 = 0x3a;
    /// Per-chunk structure references (name + boxes).
    pub const STRUCTURE_REFS: u8 = 0x77;
}

/// Build the chunk base key: `[x][z]` (+ dim byte when not overworld).
pub fn chunk_base(x: i32, z: i32, dim: u8) -> Vec<u8> {
    let mut key = Vec::with_capacity(9);
    key.extend_from_slice(&x.to_le_bytes());
    key.extend_from_slice(&z.to_le_bytes());
    if dim != 0 {
        key.push(dim);
    }
    key
}

pub fn chunk_record(x: i32, z: i32, dim: u8, tag: u8) -> Vec<u8> {
    let mut key = chunk_base(x, z, dim);
    key.push(tag);
    key
}

pub fn subchunk_key(x: i32, z: i32, dim: u8, y: i8) -> Vec<u8> {
    let mut key = chunk_record(x, z, dim, tag::SUBCHUNK_PREFIX);
    key.push(y as u8);
    key
}

pub fn version_key(x: i32, z: i32, dim: u8) -> Vec<u8> {
    chunk_record(x, z, dim, tag::VERSION)
}

pub fn data3d_key(x: i32, z: i32, dim: u8) -> Vec<u8> {
    chunk_record(x, z, dim, tag::DATA_3D)
}

pub fn digp_key(x: i32, z: i32, dim: u8) -> Vec<u8> {
    let mut key = b"digp".to_vec();
    key.extend_from_slice(&x.to_le_bytes());
    key.extend_from_slice(&z.to_le_bytes());
    if dim != 0 {
        key.push(dim);
    }
    key
}

pub const ACTOR_PREFIX: &[u8] = b"actorprefix";

pub fn actor_key(actor_id: [u8; 8]) -> Vec<u8> {
    let mut key = ACTOR_PREFIX.to_vec();
    key.extend_from_slice(&actor_id);
    key
}

/// A decoded chunk-record key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbKey<'a> {
    ChunkRecord { x: i32, z: i32, dim: u8, tag: u8 },
    Subchunk { x: i32, z: i32, dim: u8, y: i8 },
    Digp { x: i32, z: i32, dim: u8 },
    Actor { id: [u8; 8] },
    LocalPlayer,
    Player { id: &'a [u8] },
    /// Anything else: map data, villages, portals, scoreboards, metadata…
    Other,
}

/// Decode a database key into a typed view. Returns `Other` for non-chunk
/// keys rather than erroring — the LevelDB browser needs to show everything.
pub fn decode_key(key: &[u8]) -> DbKey<'_> {
    if let Some(rest) = key.strip_prefix(ACTOR_PREFIX) {
        if rest.len() == 8 {
            let mut id = [0u8; 8];
            id.copy_from_slice(rest);
            return DbKey::Actor { id };
        }
    }
    if key == b"~local_player" {
        return DbKey::LocalPlayer;
    }
    if let Some(rest) = key.strip_prefix(b"digp") {
        if let Some(decoded) = decode_chunk_base(rest) {
            return DbKey::Digp { x: decoded.x, z: decoded.z, dim: decoded.dim };
        }
        return DbKey::Other;
    }
    // Plain chunk record: 8/9/10-byte base + tag (+1 for subchunk y).
    match decode_chunk_base(key) {
        Some(base) if key.len() == base.consumed + 1 => {
            let t = key[base.consumed];
            return DbKey::ChunkRecord { x: base.x, z: base.z, dim: base.dim, tag: t };
        }
        Some(base)
            if key.len() == base.consumed + 2
                && key[base.consumed] == tag::SUBCHUNK_PREFIX =>
        {
            return DbKey::Subchunk { x: base.x, z: base.z, dim: base.dim, y: key[base.consumed + 1] as i8 };
        }
        _ => {}
    }
    if key.len() == 16 && key.iter().all(|b| b.is_ascii_hexdigit()) {
        return DbKey::Player { id: key };
    }
    DbKey::Other
}

struct DecodedBase {
    x: i32,
    z: i32,
    dim: u8,
    consumed: usize,
}

/// Try to read `[x][z][dim?]` from the front. For a 9+ byte remainder the
/// ninth byte is either a dim byte (0..=2) or the record tag; the caller
/// resolves via total key length.
fn decode_chunk_base(bytes: &[u8]) -> Option<DecodedBase> {
    if bytes.len() < 8 {
        return None;
    }
    let x = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let z = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    match bytes.len() {
        8 => Some(DecodedBase { x, z, dim: 0, consumed: 8 }),
        n if n >= 9 && bytes[8] <= 2 => Some(DecodedBase { x, z, dim: bytes[8], consumed: 9 }),
        _ => Some(DecodedBase { x, z, dim: 0, consumed: 8 }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_keys() {
        assert_eq!(chunk_base(10, -3, 0), [10, 0, 0, 0, 0xfd, 0xff, 0xff, 0xff]);
        // Nether gets the dim byte.
        assert_eq!(chunk_base(10, -3, 1).len(), 9);
        assert_eq!(chunk_base(10, -3, 1)[8], 1);
    }

    #[test]
    fn subchunk_roundtrip() {
        let key = subchunk_key(-1, 2, 0, -4);
        assert_eq!(decode_key(&key), DbKey::Subchunk { x: -1, z: 2, dim: 0, y: -4 });
        let key = subchunk_key(5, 5, 1, 19);
        assert_eq!(decode_key(&key), DbKey::Subchunk { x: 5, z: 5, dim: 1, y: 19 });
    }

    #[test]
    fn record_tags() {
        assert_eq!(decode_key(&version_key(0, 0, 0)), DbKey::ChunkRecord { x: 0, z: 0, dim: 0, tag: 0x2c });
        assert_eq!(decode_key(&data3d_key(3, 4, 0)), DbKey::ChunkRecord { x: 3, z: 4, dim: 0, tag: 0x2b });
    }

    #[test]
    fn actor_and_digp() {
        let k = actor_key([1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(decode_key(&k), DbKey::Actor { id: [1, 2, 3, 4, 5, 6, 7, 8] });
        let d = digp_key(1, 2, 1);
        assert_eq!(decode_key(&d), DbKey::Digp { x: 1, z: 2, dim: 1 });
    }
}
