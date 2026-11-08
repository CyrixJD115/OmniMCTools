//! fuzz_leveldb_key — arbitrary bytes as a Bedrock LevelDB record key.
//!
//! Invariants:
//! * `keys::decode_key` classifies any byte string without panicking.
//! * Encoder → decoder round-trips: a key built with `chunk_record` /
//!   `subchunk_key` / `digp_key` / `actor_key` decodes back to the same
//!   typed fields for coordinates derived from the fuzz input.

#![no_main]

use libfuzzer_sys::fuzz_target;
use omt_bedrock::keys::{self, DbKey};

fuzz_target!(|data: &[u8]| {
    let _ = keys::decode_key(data); // never panics; `Other` for everything unknown

    let word = |i: usize| -> i32 {
        u32::from_le_bytes([
            data.get(i).copied().unwrap_or(0),
            data.get(i + 1).copied().unwrap_or(0),
            data.get(i + 2).copied().unwrap_or(0),
            data.get(i + 3).copied().unwrap_or(0),
        ]) as i32
    };
    let x = word(0);
    let z = word(4);
    // Only real Bedrock dimensions (0 overworld / 1 nether / 2 end) are
    // round-trippable: `decode_chunk_base` treats a byte <= 2 at the base
    // boundary as the dimension, so any other dim byte is intentionally
    // indistinguishable from overworld key bytes.
    let dim = data.first().copied().unwrap_or(0);
    let tag = data.get(8).copied().unwrap_or(0);
    let y = data.get(9).copied().unwrap_or(0) as i8;

    if dim <= 2 {
        if !(dim == 0 && tag <= 2) {
            // Overworld tags <= 2 would be read back as a dimension byte —
            // no real record uses tags that low (see keys::tag constants).
            assert_eq!(
                keys::decode_key(&keys::chunk_record(x, z, dim, tag)),
                DbKey::ChunkRecord { x, z, dim, tag },
                "chunk_record round-trip"
            );
        }
        assert_eq!(
            keys::decode_key(&keys::subchunk_key(x, z, dim, y)),
            DbKey::Subchunk { x, z, dim, y },
            "subchunk_key round-trip"
        );
        assert_eq!(
            keys::decode_key(&keys::digp_key(x, z, dim)),
            DbKey::Digp { x, z, dim },
            "digp_key round-trip"
        );
    }
    let id: [u8; 8] = data
        .get(..8)
        .and_then(|s| s.try_into().ok())
        .unwrap_or([0u8; 8]);
    assert_eq!(keys::decode_key(&keys::actor_key(id)), DbKey::Actor { id });
});
