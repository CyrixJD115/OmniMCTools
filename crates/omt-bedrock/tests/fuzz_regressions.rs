//! Regression tests for `fuzz_subchunk_codec` crashers (see fuzz/README.md):
//! lenient-decoded non-canonical shapes must re-encode to something the
//! decoder accepts again (decode⇄encode round-trip or typed error).

use omt_bedrock::{Data3d, Subchunk};

fn palette_entry_air() -> Vec<u8> {
    // NBT root: compound "" { "name": "air" } — Bedrock NBT: little-endian
    // string lengths (fixture-verified `0a 00 00 08 04 00 "name" …`).
    vec![
        0x0a, 0x00, 0x00, 0x08, 0x04, 0x00, b'n', b'a', b'm', b'e', 0x03, 0x00, b'a', b'i', b'r',
        0x00,
    ]
}

#[test]
fn packed_storage_with_zero_palette_is_a_typed_error() {
    // v8, 1 storage, bits=2, 1024 packed bytes, count = 0, trailing garbage.
    // Found by fuzz_subchunk_codec: serialize turned this into an invalid
    // bits==0 storage, so decode⇄encode crashed instead of erroring.
    let mut buf = vec![0x08, 0x01, 0x04];
    buf.extend(vec![0u8; 1024]);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&[0xff; 8]);
    let err = Subchunk::parse(&buf, None).unwrap_err();
    assert!(
        matches!(err, omt_bedrock::subchunk::SubchunkError::EmptyPalette),
        "want EmptyPalette, got {err}"
    );
}

#[test]
fn data3d_zero_palette_is_a_typed_error() {
    // 512-byte heightmap + one packed slot (bits=2) with count = 0.
    let mut buf = vec![0u8; 512];
    buf.push(2 << 1);
    buf.extend(vec![0u8; 1024]);
    buf.extend_from_slice(&0u32.to_le_bytes());
    let err = Data3d::parse(&buf).unwrap_err();
    assert!(err.to_string().contains("empty biome palette"), "{err}");
}

#[test]
fn noncanonical_single_entry_storage_roundtrips() {
    // bits=2 with count=1: self-consistent but not what the game writes.
    // The serializer must keep the raw bits (not canonicalize to bits==0)
    // so the re-encode parses back to the same value.
    let mut buf = vec![0x09, 0x01, 0x05]; // v9, 1 storage, y=5
    buf.push(2 << 1);
    buf.extend(vec![0x40u8; 1024]); // 2-bit indices
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(&palette_entry_air());
    let sub = Subchunk::parse(&buf, None).unwrap();
    let re = sub.serialize();
    let re_parsed = Subchunk::parse(&re, None).unwrap();
    assert_eq!(re_parsed, sub, "decode⇄encode must round-trip");
}

#[test]
fn noncanonical_single_biome_data3d_roundtrips() {
    // data3D: heightmap + one packed slot (bits=2) with exactly one palette
    // id and indices — must keep its packed form on re-encode.
    let mut buf = vec![0u8; 512];
    buf.push(2 << 1);
    buf.extend(vec![0x40u8; 1024]);
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(&7i32.to_le_bytes());
    let d3 = Data3d::parse(&buf).unwrap();
    let re = d3.serialize();
    assert_eq!(Data3d::parse(&re).unwrap(), d3, "decode⇄encode must round-trip");
}

#[test]
fn canonical_single_entry_storage_unchanged() {
    // The fixture-canonical bits==0 shape: header 0x00, single entry, no
    // count prefix — must still encode byte-identically.
    let mut buf = vec![0x08, 0x01, 0x00];
    buf.extend_from_slice(&palette_entry_air());
    let sub = Subchunk::parse(&buf, None).unwrap();
    assert_eq!(sub.serialize(), buf, "canonical bits==0 must re-encode byte-identically");
}
