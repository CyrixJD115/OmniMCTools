//! LevelDB key codec for Bedrock worlds.
//!
//! Layout **verified against the 1.26.52 `world_ref` fixtures** (see
//! `examples/probe.rs`; byte evidence in the M1 notes):
//! - chunk base: `[i32 LE x][i32 LE z]`, plus an **`i32 LE` dimension id**
//!   (`1` nether / `2` end) for non-overworld chunks (overworld has none),
//!   plus a one-byte record tag. Verified 2026-11-16 on a real world with an
//!   explored Nether (13/14-byte keys `…01 00 00 00 2f yy`); the earlier
//!   "single dim byte" note was never exercised (overworld-only fixtures).
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
    /// Pre-1.18 2D data: heightmap + 256 biome ids (`Data2D`).
    pub const DATA_2D_LEGACY: u8 = 0x2d;
    /// Older 2D data: heightmap + 256 × (biome id, rgb) (`Data2DLegacy`).
    pub const DATA_2D_OLD: u8 = 0x2e;
    /// Pre-1.0 whole-column terrain (`LegacyTerrain`).
    pub const LEGACY_TERRAIN: u8 = 0x30;
    /// Chunk version before 1.16.100 (`VersionOld`).
    pub const VERSION_OLD: u8 = 0x76;
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

/// Build the chunk base key: `[x][z]` (+ `i32 LE` dim when not overworld).
pub fn chunk_base(x: i32, z: i32, dim: u8) -> Vec<u8> {
    let mut key = Vec::with_capacity(12);
    key.extend_from_slice(&x.to_le_bytes());
    key.extend_from_slice(&z.to_le_bytes());
    if dim != 0 {
        key.extend_from_slice(&(dim as i32).to_le_bytes());
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
    key.extend_from_slice(&chunk_base(x, z, dim));
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
    if let Some(rest) = key.strip_prefix(ACTOR_PREFIX)
        && rest.len() == 8 {
            let mut id = [0u8; 8];
            id.copy_from_slice(rest);
            return DbKey::Actor { id };
        }
    if key == b"~local_player" {
        return DbKey::LocalPlayer;
    }
    if let Some(rest) = key.strip_prefix(b"digp") {
        return match rest.len() {
            8 => {
                let (x, z) = xz(rest);
                DbKey::Digp { x, z, dim: 0 }
            }
            12 => match dim_of(&rest[8..12]) {
                Some(dim) => {
                    let (x, z) = xz(rest);
                    DbKey::Digp { x, z, dim }
                }
                None => DbKey::Other,
            },
            _ => DbKey::Other,
        };
    }
    // Plain chunk record: base (8 or 12 bytes) + tag (+1 y for subchunks).
    // Total length disambiguates: 9/10 overworld, 13/14 other dimensions.
    let (dim, rest) = match key.len() {
        9 | 10 => (Some(0), &key[8..]),
        13 | 14 => (dim_of(&key[8..12]), &key[12..]),
        _ => (None, &key[0..0]),
    };
    if let Some(dim) = dim {
        let (x, z) = xz(key);
        match rest {
            [t] if is_record_tag(*t) => return DbKey::ChunkRecord { x, z, dim, tag: *t },
            [tag::SUBCHUNK_PREFIX, y] => return DbKey::Subchunk { x, z, dim, y: *y as i8 },
            _ => {}
        }
    }
    if key.len() == 16 && key.iter().all(|b| b.is_ascii_hexdigit()) {
        return DbKey::Player { id: key };
    }
    DbKey::Other
}

fn xz(bytes: &[u8]) -> (i32, i32) {
    let x = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let z = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    (x, z)
}

/// A non-overworld dimension id (`i32 LE`, 1 or 2).
fn dim_of(bytes: &[u8]) -> Option<u8> {
    match i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) {
        d @ 1..=2 => Some(d as u8),
        _ => None,
    }
}

/// Chunk record tags documented for current Bedrock (0x2B..=0x41, 0x76,
/// 0x77). Restricting the tag keeps arbitrary 9/10/13/14-byte names (e.g.
/// player UUID keys, `Overworld`) from decoding as chunk records.
fn is_record_tag(t: u8) -> bool {
    matches!(t, 0x2b..=0x41 | 0x76 | 0x77)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_keys() {
        assert_eq!(chunk_base(10, -3, 0), [10, 0, 0, 0, 0xfd, 0xff, 0xff, 0xff]);
        // Nether gets an i32 LE dimension id.
        assert_eq!(chunk_base(10, -3, 1).len(), 12);
        assert_eq!(&chunk_base(10, -3, 1)[8..], &[1, 0, 0, 0]);
        assert_eq!(&chunk_base(0, 0, 2)[8..], &[2, 0, 0, 0]);
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
        assert_eq!(decode_key(&digp_key(-7, 9, 0)), DbKey::Digp { x: -7, z: 9, dim: 0 });
    }

    #[test]
    fn real_nether_keys() {
        // Byte-exact keys observed in a real world with an explored Nether.
        let sub = [0u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0x2f, 1];
        assert_eq!(decode_key(&sub), DbKey::Subchunk { x: 0, z: 0, dim: 1, y: 1 });
        let ver = [0u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0x2c];
        assert_eq!(decode_key(&ver), DbKey::ChunkRecord { x: 0, z: 0, dim: 1, tag: 0x2c });
        // Round trips for every dimension.
        for dim in 0..=2u8 {
            assert_eq!(decode_key(&subchunk_key(-3, 4, dim, -4)), DbKey::Subchunk { x: -3, z: 4, dim, y: -4 });
            assert_eq!(decode_key(&chunk_record(5, -6, dim, 0x31)), DbKey::ChunkRecord { x: 5, z: -6, dim, tag: 0x31 });
        }
    }
}
