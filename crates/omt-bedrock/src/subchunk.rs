//! Bedrock subchunk codec (palette format v8/v9).
//!
//! On-disk layout, verified byte-by-byte against the 1.26.52 fixtures:
//!
//! ```text
//! u8   version (8 or 9)
//! u8   storage_count
//! u8   y_index          (v9 only; repeats the key's Y byte)
//! per storage:
//!   u8  header  = bits_per_block << 1 | is_runtime
//!   [4096 blocks packed at `bits` bits, LSB-first, little-endian words]
//!       (absent when bits == 0)
//!   palette:
//!     u32 LE count
//!     count × NBT entry: sequence of named tags terminated by TAG_End
//!       "name"    TAG_String   e.g. "minecraft:stone"
//!       "states"  TAG_Compound (optional) block-state properties
//!       "version" TAG_Int      (optional, data-driven block version)
//!     is_runtime palettes instead store u32 runtime ids — not present in
//!     disk saves; rejected with a clear error.
//! ```
//!
//! Block index order is **XZY** (`(x << 8) | (z << 4) | y`, y fastest),
//! distinct from Java's YZX sections (see `crate::store` for the accessor
//! that hides this). **Verified against the UMT binary** (REA pass
//! 2026-11-03, findings Task D): UMT's Bedrock universal-chunk
//! `getBlock` (`0x4A9DF0`) and `setBlock` (`0x4AE610`) both compute
//! `idx = ((x & 0xF) * 0x10 + (z & 0xF)) * 0x10 + (y & 0xF)` with
//! `slot = floorDiv(worldY, 16)` — decompiles archived as
//! `Source/RE/umt_re_evidence/decompiled/rea_func_4a9df0.c` /
//! `rea_func_4ae610.c`. The 1.26.52 fixtures agree (M1 byte-exact
//! round-trip under this interpretation).

use omt_nbt::{ByteOrder, Reader, Tag};
use omt_world::BlockState;



#[derive(Debug, Clone, PartialEq)]
pub struct SubchunkStorage {
    pub palette: Vec<BlockState>,
    /// Palette entries as their original NBT roots (including "version" and
    /// any future fields), preserved verbatim for byte-faithful re-encode.
    /// `None` for storages built programmatically; the serializer then
    /// rebuilds entries from `palette`.
    pub raw_entries: Option<Vec<omt_nbt::Tag>>,
    /// 4096 palette indices in XZY order. `None` means bits == 0 (all blocks
    /// are palette[0]).
    pub indices: Option<Vec<u16>>,
    /// Raw header bits, preserved for faithful re-encode.
    pub bits: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Subchunk {
    pub version: u8,
    pub y: i8,
    pub storages: Vec<SubchunkStorage>,
}

#[derive(Debug, thiserror::Error)]
pub enum SubchunkError {
    #[error("unsupported subchunk version {0} (expected 8 or 9)")]
    UnsupportedVersion(u8),
    #[error("runtime-ID palettes are not stored on disk; found one (corrupt or network payload?)")]
    RuntimePalette,
    #[error("bad bits-per-block {0} (max 16)")]
    BadBits(u8),
    #[error("storage count {0} exceeds 2")]
    TooManyStorages(u8),
    #[error("y mismatch: key says {key}, payload says {payload}")]
    YMismatch { key: i8, payload: i8 },
    #[error("NBT error in palette: {0}")]
    Nbt(#[from] omt_nbt::Error),
    #[error("palette entry #{0} has no \"name\" string")]
    PaletteNoName(usize),
    #[error("packed storage declares 4096 indices but an empty palette (nothing to resolve against)")]
    EmptyPalette,
}

#[derive(Debug, thiserror::Error)]
pub enum Data3dError {
    #[error("data3D too short: {0} bytes")]
    TooShort(usize),
    #[error("data3D subchunk {slot}: {reason}")]
    Subchunk { slot: usize, reason: String },
    #[error("NBT error: {0}")]
    Nbt(#[from] omt_nbt::Error),
}

/// One subchunk slot's biome record inside data3D — shared model type
/// (see `omt_world::BedrockBiomeSlot` for the semantics).
pub use omt_world::BedrockBiomeSlot as SubchunkBiomes;

/// Decoded `0x2B` record: 512-byte heightmap + per-subchunk biome storages.
#[derive(Debug, Clone, PartialEq)]
pub struct Data3d {
    /// 256 heights (u16 LE), column order `z*16 + x` (z-major, x fastest —
    /// probe-verified, see PLAN.md "Verified 1.26.52 format facts").
    pub heightmap: Vec<u16>,
    /// One entry per subchunk slot, bottom to top; length is however many
    /// slots the world wrote (16 in the fixtures).
    pub subchunks: Vec<SubchunkBiomes>,
}

impl Data3d {
    pub fn parse(buf: &[u8]) -> Result<Self, Data3dError> {
        if buf.len() < 512 {
            return Err(Data3dError::TooShort(buf.len()));
        }
        let mut heightmap = Vec::with_capacity(256);
        for i in 0..256 {
            let b = &buf[i * 2..i * 2 + 2];
            heightmap.push(u16::from_le_bytes([b[0], b[1]]));
        }
        let mut r = Reader::with_offset(buf, ByteOrder::Little, 512);
        let mut subchunks = Vec::new();
        while r.remaining() > 0 {
            let slot = subchunks.len();
            let header = r.u8().map_err(|e| Data3dError::Subchunk { slot, reason: e.to_string() })?;
            if header == 0xff {
                subchunks.push(SubchunkBiomes::NonExistent);
                continue;
            }
            let bits = header >> 1;
            if bits > 16 {
                return Err(Data3dError::Subchunk { slot, reason: format!("bad bits {bits}") });
            }
            if bits == 0 {
                // Single biome; count omitted (1.18.31+ amendment).
                let mut id = [0u8; 4];
                id.copy_from_slice(
                    r.take(4).map_err(|e| Data3dError::Subchunk { slot, reason: e.to_string() })?,
                );
                subchunks.push(SubchunkBiomes::uniform(i32::from_le_bytes(id)));
                continue;
            }
            let raw = r
                .take(packed_len(bits))
                .map_err(|e| Data3dError::Subchunk { slot, reason: e.to_string() })?;
            let indices = unpack_bits(raw, bits);
            let count = r
                .u32()
                .map_err(|e| Data3dError::Subchunk { slot, reason: e.to_string() })?
                as usize;
            if count == 0 {
                // Malformed: indices declared but no palette to resolve them
                // (found by fuzz_subchunk_codec; serializer can't express it).
                return Err(Data3dError::Subchunk { slot, reason: "empty biome palette".into() });
            }
            let mut palette = Vec::with_capacity(count.min(256));
            for _ in 0..count {
                let mut b = [0u8; 4];
                b.copy_from_slice(
                    r.take(4).map_err(|e| Data3dError::Subchunk { slot, reason: e.to_string() })?,
                );
                palette.push(i32::from_le_bytes(b));
            }
            subchunks.push(SubchunkBiomes::Storage { palette, indices: Some(indices), bits });
        }
        Ok(Self { heightmap, subchunks })
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(512 + self.subchunks.len() * 8);
        for h in &self.heightmap {
            out.extend_from_slice(&h.to_le_bytes());
        }
        for sc in &self.subchunks {
            match sc {
                SubchunkBiomes::NonExistent => out.push(0xff),
                SubchunkBiomes::Storage { palette, indices, bits } => {
                    if palette.len() <= 1 && indices.is_none() {
                        // Canonical uniform slot. A Storage carrying indices
                        // (lenient parse of non-canonical but valid data)
                        // must keep its bits so decode⇄encode round-trips.
                        out.push(0x01);
                        out.extend_from_slice(&palette.first().copied().unwrap_or(0).to_le_bytes());
                    } else {
                        out.push(bits << 1 | 1);
                        pack_bits(&mut out, indices.as_deref().unwrap_or(&[0; 0]), *bits);
                        out.extend_from_slice(&(palette.len() as u32).to_le_bytes());
                        for id in palette {
                            out.extend_from_slice(&id.to_le_bytes());
                        }
                    }
                }
            }
        }
        out
    }

    /// Biome at block coordinates within the column (any Y in the covered
    /// range), resolving NonExistent slots from the nearest storage below.
    pub fn biome_at(&self, x: usize, y: usize, z: usize) -> Option<i32> {
        let slot = y / 16;
        let sc = self.subchunks.get(slot)?;
        let (palette, indices) = match sc {
            SubchunkBiomes::NonExistent => {
                // fall through to the storage below this slot
                let mut s = slot;
                loop {
                    if s == 0 {
                        return None;
                    }
                    s -= 1;
                    if let SubchunkBiomes::Storage { palette, indices, .. } = &self.subchunks[s] {
                        break (palette, indices.as_ref());
                    }
                }
            }
            SubchunkBiomes::Storage { palette, indices, .. } => (palette, indices.as_ref()),
        };
        let idx = indices
            .map(|ix| ix.get(block_index_xzy(x, y % 16, z)).copied().unwrap_or(0) as usize)
            .unwrap_or(0);
        palette.get(idx).copied()
    }
}

impl Subchunk {
    /// Decode a subchunk payload; `key_y` (when provided) is checked against
    /// the embedded v9 Y byte as a consistency guard.
    pub fn parse(buf: &[u8], key_y: Option<i8>) -> Result<Self, SubchunkError> {
        if buf.is_empty() {
            return Err(SubchunkError::UnsupportedVersion(0));
        }
        let version = buf[0];
        if version != 8 && version != 9 {
            return Err(SubchunkError::UnsupportedVersion(version));
        }
        if buf.len() < 2 {
            return Err(SubchunkError::UnsupportedVersion(version));
        }
        let storage_count = buf[1];
        if storage_count > 2 {
            return Err(SubchunkError::TooManyStorages(storage_count));
        }
        let mut r = Reader::with_offset(buf, ByteOrder::Little, 2);
        let y = if version >= 9 { r.u8()? as i8 } else { 0 };
        if let Some(ky) = key_y
            && version >= 9 && y != ky {
                return Err(SubchunkError::YMismatch { key: ky, payload: y });
            }
        let mut storages = Vec::with_capacity(storage_count as usize);
        for _ in 0..storage_count {
            storages.push(parse_storage(&mut r)?);
        }
        Ok(Self { version, y, storages })
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16);
        out.push(self.version);
        out.push(self.storages.len() as u8);
        if self.version >= 9 {
            out.push(self.y as u8);
        }
        for storage in &self.storages {
            serialize_storage(&mut out, storage);
        }
        out
    }

    /// First (block) storage's block index in XZY order.
    pub fn block_index(&self, x: usize, y: usize, z: usize) -> Option<u16> {
        let storage = self.storages.first()?;
        let raw = match (&storage.indices, storage.palette.len()) {
            (_, 1) => 0,
            (Some(idx), _) => *idx.get(block_index_xzy(x, y, z))?,
            (None, _) => 0,
        };
        Some(raw)
    }

    pub fn block(&self, x: usize, y: usize, z: usize) -> Option<&BlockState> {
        let storage = self.storages.first()?;
        storage.palette.get(self.block_index(x, y, z)? as usize)
    }
}

/// Bedrock block ordering: x-major, then z, then y — `(x << 8) | (z << 4) | y`.
///
/// UMT-verified (REA findings Task D, 2026-11-03): `getBlock`/`setBlock` at
/// `0x4A9DF0`/`0x4AE610` use exactly this nesting; see the module docs.
#[inline]
pub fn block_index_xzy(x: usize, y: usize, z: usize) -> usize {
    (x << 8) | (z << 4) | y
}

fn parse_storage(r: &mut Reader<'_>) -> Result<SubchunkStorage, SubchunkError> {
    let header = r.u8()?;
    let bits = header >> 1;
    let is_runtime = header & 1 != 0;
    if bits > 16 {
        return Err(SubchunkError::BadBits(bits));
    }
    if is_runtime {
        return Err(SubchunkError::RuntimePalette);
    }
    let indices = if bits == 0 {
        None
    } else {
        let byte_len = packed_len(bits);
        let raw = r.take(byte_len)?;
        Some(unpack_bits(raw, bits))
    };
    // bits == 0: exactly one palette entry, written WITHOUT a count prefix
    // (fixture-verified: header 0x00 then `0a 00 00 08 …` directly).
    let count = if bits == 0 { 1 } else { r.u32()? as usize };
    if bits > 0 && count == 0 {
        // Malformed: 4096 indices declared but no palette to resolve them
        // against. The serializer cannot express this shape (found by
        // fuzz_subchunk_codec — see fuzz/README.md), so reject it here.
        return Err(SubchunkError::EmptyPalette);
    }
    let mut palette = Vec::with_capacity(count.min(65536));
    let mut raw_entries = Vec::with_capacity(count.min(65536));
    for i in 0..count {
        // Each entry is a full NBT root: `0a 00 00` (TAG_Compound, empty
        // name) + "name"/"states"/"version" children + 0x00 terminator.
        // Verified against fixture bytes: `0a 00 00 08 04 00 "name" ...`.
        let Some((_, entry)) = r.read_named()? else {
            return Err(SubchunkError::PaletteNoName(i));
        };
        let Tag::Compound(fields) = &entry else {
            return Err(SubchunkError::PaletteNoName(i));
        };
        raw_entries.push(entry.clone());
        let mut name: Option<String> = None;
        let mut properties: Vec<(String, String)> = Vec::new();
        for (key, tag) in fields {
            match (key.as_str(), tag) {
                ("name", Tag::String(v)) => name = Some(v.clone()),
                ("states", Tag::Compound(states)) => {
                    for (pk, pv) in states {
                        if let Tag::String(v) = pv {
                            properties.push((pk.clone(), v.clone()));
                        } else {
                            // Non-string state (int/byte) — Bedrock data-driven
                            // blocks carry several scalar types; keep the SNBT
                            // form so conversion can parse it back.
                            properties.push((pk.clone(), omt_nbt::snbt::to_snbt(pv)));
                        }
                    }
                }
                _ => {} // "version" and future keys
            }
        }
        let name = name.ok_or(SubchunkError::PaletteNoName(i))?;
        palette.push(BlockState { name, properties });
    }
    Ok(SubchunkStorage { palette, raw_entries: Some(raw_entries), indices, bits })
}

fn serialize_storage(out: &mut Vec<u8>, storage: &SubchunkStorage) {
    // Canonical single-entry storages carry bits == 0 and no indices. If a
    // decoded storage *does* carry indices with a small palette (lenient
    // parse of a non-canonical but self-consistent payload), keep its raw
    // bits so decode⇄encode round-trips (fuzz_subchunk_codec invariant).
    let bits = if storage.indices.is_some() {
        storage
            .bits
            .max(min_bits(storage.palette.len().saturating_sub(1)))
            .max(1)
    } else {
        0
    };
    out.push(bits << 1);
    if bits > 0 {
        let idx = storage.indices.as_ref().expect("decoded storages with >1 palette entries carry indices");
        pack_bits(out, idx, bits);
    }
    // bits == 0 writes its single palette entry with no count prefix.
    if bits > 0 {
        let count = storage.raw_entries.as_ref().map_or(storage.palette.len(), Vec::len);
        out.extend_from_slice(&(count as u32).to_le_bytes());
    }
    if let Some(entries) = &storage.raw_entries {
        // Byte-faithful path: re-emit the original NBT roots.
        for entry in entries {
            out.push(entry.id());
            omt_nbt::write::push_string_pub(out, "");
            omt_nbt::write::push_payload_pub(out, entry);
        }
        return;
    }
    for state in &storage.palette {
        // Full NBT root per entry: compound with empty name.
        out.push(10);
        out.extend_from_slice(&[0, 0]); // empty root name
        out.push(8); // TAG_String "name"
        push_len_prefixed(out, "name");
        push_len_prefixed(out, &state.name);
        if !state.properties.is_empty() {
            out.push(10); // TAG_Compound "states"
            push_len_prefixed(out, "states");
            for (k, v) in &state.properties {
                out.push(8);
                push_len_prefixed(out, k);
                push_len_prefixed(out, v);
            }
            out.push(0); // end states compound
        }
        out.push(0); // end entry compound
    }
}

fn push_len_prefixed(out: &mut Vec<u8>, s: &str) {
    let mut buf = Vec::with_capacity(s.len());
    omt_nbt::string::encode_modified_utf8(s, &mut buf);
    out.extend_from_slice(&(buf.len() as u16).to_le_bytes());
    out.extend_from_slice(&buf);
}

fn min_bits(max_index: usize) -> u8 {
    let mut bits = 1u8;
    while (1usize << bits) <= max_index {
        bits += 1;
    }
    bits.min(16)
}

/// Unpack `bits`-wide values from Bedrock's **word-aligned** packed layout:
/// values are packed LSB-first into little-endian 32-bit words, each word
/// holding `floor(32/bits)` values (leftover high bits unused). For
/// power-of-two widths this is identical to a continuous bit stream.
pub fn unpack_bits(raw: &[u8], bits: u8) -> Vec<u16> {
    assert!(bits > 0 && bits <= 16, "bits must be 1..=16");
    let bits = bits as usize;
    let vpw = 32 / bits;
    let mask = (1u32 << bits) - 1;
    let mut out = Vec::with_capacity(4096);
    for chunk in raw.as_chunks::<4>().0 {
        let word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        for j in 0..vpw {
            out.push(((word >> (j * bits)) & mask) as u16);
        }
    }
    out.resize(4096, 0);
    out
}

/// Byte length of 4096 word-aligned values at `bits` width.
pub fn packed_len(bits: u8) -> usize {
    if bits == 0 {
        return 0;
    }
    let vpw = 32 / bits as usize;
    4096usize.div_ceil(vpw) * 4
}

/// Pack 4096 `bits`-wide values into the word-aligned layout.
pub fn pack_bits(out: &mut Vec<u8>, values: &[u16], bits: u8) {
    assert!(bits > 0 && bits <= 16, "bits must be 1..=16");
    let bits = bits as usize;
    let vpw = 32 / bits;
    let words = 4096usize.div_ceil(vpw);
    out.resize(out.len() + words * 4, 0);
    let start = out.len() - words * 4;
    for (i, chunk) in out[start..].as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let mut word = 0u32;
        for j in 0..vpw {
            let idx = i * vpw + j;
            let v = values.get(idx).copied().unwrap_or(0) as u32;
            word |= v << (j * bits);
        }
        chunk.copy_from_slice(&word.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone() -> BlockState {
        BlockState { name: "minecraft:stone".into(), properties: vec![] }
    }

    fn strip_raw(mut sc: Subchunk) -> Subchunk {
        for s in &mut sc.storages {
            s.raw_entries = None;
        }
        sc
    }

    #[test]
    fn single_palette_roundtrip() {
        let sc = Subchunk {
            version: 9,
            y: 3,
            storages: vec![SubchunkStorage {
                palette: vec![stone()],
                raw_entries: None,
                indices: None,
                bits: 0,
            }],
        };
        let bytes = sc.serialize();
        assert_eq!(&bytes[..4], &[9, 1, 3, 0]);
        let back = Subchunk::parse(&bytes, Some(3)).unwrap();
        assert_eq!(strip_raw(back), sc);
        // And the raw-entry path must be stable across a second pass.
        let raw = Subchunk::parse(&bytes, Some(3)).unwrap();
        assert_eq!(Subchunk::parse(&raw.serialize(), Some(3)).unwrap(), raw);
    }

    #[test]
    fn multi_palette_roundtrip() {
        let palette = vec![
            BlockState { name: "minecraft:air".into(), properties: vec![] },
            stone(),
            BlockState {
                name: "minecraft:oak_leaves".into(),
                properties: vec![("persistent".into(), "true".into())],
            },
        ];
        let mut indices = vec![0u16; 4096];
        indices[0] = 1;
        indices[1] = 2;
        indices[4095] = 1;
        let sc = Subchunk {
            version: 9,
            y: -4,
            storages: vec![SubchunkStorage { palette, raw_entries: None, indices: Some(indices), bits: 2 }],
        };
        let bytes = sc.serialize();
        let back = Subchunk::parse(&bytes, Some(-4)).unwrap();
        assert_eq!(strip_raw(back.clone()), sc);
        assert_eq!(Subchunk::parse(&back.serialize(), Some(-4)).unwrap(), back);
        assert_eq!(back.block(0, 0, 0), Some(&stone()));
    }

    #[test]
    fn bits_packing_roundtrip_all_widths() {
        for bits in [1u8, 2, 3, 4, 5, 6, 8, 12, 16] {
            let values: Vec<u16> = (0..4096u32).map(|i| (i % (1u32 << bits)) as u16).collect();
            let mut packed = Vec::new();
            pack_bits(&mut packed, &values, bits);
            let unpacked = unpack_bits(&packed, bits);
            assert_eq!(unpacked, values, "bits={bits}");
            // canonical word-aligned size
            assert_eq!(packed.len(), packed_len(bits), "bits={bits}");
        }
    }

    #[test]
    fn data3d_roundtrip() {
        // Mixed record: uniform slot, 2-biome slot, nonexistent tail.
        let mut indices = vec![0u16; 4096];
        indices[112] = 1; // xzy(0,0,7) = 7*16
        let d = Data3d {
            heightmap: (0..256).map(|i| 128 + i as u16 % 32).collect(),
            subchunks: vec![
                SubchunkBiomes::uniform(30),
                SubchunkBiomes::Storage {
                    palette: vec![30, 188],
                    indices: Some(indices),
                    bits: 1,
                },
                SubchunkBiomes::NonExistent,
            ],
        };
        let bytes = d.serialize();
        // 512 + [01 id] + [03 packed(512B) count 2×i32] + [ff]
        assert_eq!(bytes.len(), 512 + 5 + (1 + 512 + 4 + 8) + 1);
        assert_eq!(bytes[512], 0x01);
        assert_eq!(&bytes[513..517], &30i32.to_le_bytes());
        assert_eq!(bytes[517], 0x03);
        let back = Data3d::parse(&bytes).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.biome_at(0, 16, 7), Some(188));
        assert_eq!(back.biome_at(0, 16, 0), Some(30));
        // NonExistent slot resolves from the storage below.
        assert_eq!(back.biome_at(0, 32, 7), Some(188));
    }

    /// Byte-exact regression from the 1.26.52 fixture: a fully-uniform
    /// column serializes as 12 × `01 <i32 biome>` + `ff` markers.
    #[test]
    fn data3d_fixture_uniform_shape() {
        let d = Data3d {
            heightmap: vec![134; 256],
            subchunks: std::iter::repeat_n(SubchunkBiomes::uniform(30), 12)
                .chain(std::iter::repeat_n(SubchunkBiomes::NonExistent, 4))
                .collect(),
        };
        let bytes = d.serialize();
        assert_eq!(bytes.len(), 512 + 12 * 5 + 4);
        assert_eq!(&bytes[512..517], &[0x01, 0x1e, 0x00, 0x00, 0x00]);
        assert_eq!(&bytes[bytes.len() - 4..], &[0xff; 4]);
        assert_eq!(Data3d::parse(&bytes).unwrap(), d);
    }
}
