//! Bit-packing layouts shared by the Java and Bedrock codecs.
//!
//! Two different packing families exist and must never be mixed:
//!
//! - **Java (1.16+, our read floor)**: values are packed LSB-first into
//!   64-bit longs and **never span long boundaries** — each long holds
//!   `floor(64 / bits)` values and leftover high bits are padding.
//!   (`wiki.vg` Chunk Format "Data" — the pre-1.16 continuous stream is out
//!   of scope for a latest-only tool.)
//! - **Bedrock**: values are packed LSB-first into 32-bit LE words,
//!   word-aligned (`omt_bedrock::subchunk::pack_bits`); keep the two apart.

// Index orders for 16×16×16 sections, named most-significant-axis first:
// Java sections are YZX (`y << 8 | z << 4 | x`), Bedrock subchunks are XZY
// (`x << 8 | z << 4 | y`, UMT-verified — see omt-bedrock subchunk docs).
// These helpers convert between the two orders losslessly.

/// Convert 4096 XZY-ordered indices (Bedrock) to YZX order (Java/universal).
pub fn permute_xzy_to_yzx(xzy: &[u16]) -> Vec<u16> {
    permute(xzy, |x, y, z| (x << 8) | (z << 4) | y, |x, y, z| (y << 8) | (z << 4) | x)
}

/// Convert 4096 YZX-ordered indices (Java/universal) to XZY order (Bedrock).
pub fn permute_yzx_to_xzy(yzx: &[u16]) -> Vec<u16> {
    permute(yzx, |x, y, z| (y << 8) | (z << 4) | x, |x, y, z| (x << 8) | (z << 4) | y)
}

fn permute(src: &[u16], src_at: fn(usize, usize, usize) -> usize, dst_at: fn(usize, usize, usize) -> usize) -> Vec<u16> {
    let mut out = vec![0u16; 4096];
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                out[dst_at(x, y, z)] = src.get(src_at(x, y, z)).copied().unwrap_or(0);
            }
        }
    }
    out
}

/// Bits per entry for a Java **block** palette: minimum 4 (vanilla), grows
/// past the palette size; global palettes beyond 16 bits are unsupported.
pub fn java_block_bits(palette_len: usize) -> u32 {
    if palette_len <= 1 {
        return 0;
    }
    min_bits_at_least(palette_len, 4)
}

/// Bits per entry for a Java **biome** palette: minimum 1, cells are 4³ = 64.
pub fn java_biome_bits(palette_len: usize) -> u32 {
    if palette_len <= 1 {
        return 0;
    }
    min_bits_at_least(palette_len, 1)
}

fn min_bits_at_least(palette_len: usize, floor: u32) -> u32 {
    let mut bits = floor;
    while (1usize << bits) < palette_len && bits < 16 {
        bits += 1;
    }
    bits
}

/// Unpack `values` entries of `bits` width from Java's non-spanning layout.
/// Entry *i* lives in long `i / (64/bits)` at shift `(i % (64/bits)) * bits`,
/// LSB-first. Truncated input pads with 0 (lenient like the codecs).
pub fn unpack_java_packed(packed: &[i64], values: usize, bits: u32) -> Vec<u16> {
    if bits == 0 || values == 0 {
        return vec![0u16; values];
    }
    let bits = bits as usize;
    let per_long = 64 / bits;
    let mask = (1u64 << bits) - 1;
    let mut out = Vec::with_capacity(values);
    for i in 0..values {
        let word = packed.get(i / per_long).copied().unwrap_or(0) as u64;
        out.push(((word >> ((i % per_long) * bits)) & mask) as u16);
    }
    out
}

/// Unpack a Java block-state `data` long array (4096 entries).
pub fn unpack_java_indices(packed: &[i64], palette_len: usize) -> Vec<u16> {
    unpack_java_packed(packed, 4096, java_block_bits(palette_len))
}

/// Unpack a Java per-section biome `data` long array (64 cells, index
/// `(y << 4) | (z << 2) | x`).
pub fn unpack_java_biome_cells(packed: &[i64], palette_len: usize) -> Vec<u16> {
    unpack_java_packed(packed, 64, java_biome_bits(palette_len))
}

/// Unpack one Java `Heightmaps` entry: 256 columns at 9 bits (37 longs,
/// 7 values per long). Values are "height above world bottom + 1"
/// (world bottom = Y −64), the same numeric encoding Bedrock stores in the
/// data3D heightmap — see PLAN.md "Verified 1.26.52 format facts".
pub fn unpack_java_heightmap(packed: &[i64]) -> Vec<u16> {
    unpack_java_packed(packed, 256, 9)
}

/// Pack `values` entries at `bits` width into Java's non-spanning layout:
/// entry *i* lives in long `i / (64/bits)` at shift `(i % (64/bits)) * bits`,
/// LSB-first; longs are padded with zero high bits. The inverse of
/// [`unpack_java_packed`] (verified by round-trip tests below). Values are
/// truncated to `bits` width.
pub fn pack_java_packed(values: &[u16], bits: u32) -> Vec<i64> {
    if bits == 0 || values.is_empty() {
        return Vec::new();
    }
    let bits = bits as usize;
    let per_long = 64 / bits;
    let mask = ((1u64 << bits) - 1) as i64;
    let mut out = vec![0i64; values.len().div_ceil(per_long)];
    for (i, &v) in values.iter().enumerate() {
        out[i / per_long] |= (v as i64 & mask) << ((i % per_long) * bits);
    }
    out
}

/// Pack a Java block-state `data` long array from 4096 palette indices
/// (YZX order, already in Java order — no permutation here).
pub fn pack_java_indices(indices: &[u16], palette_len: usize) -> Vec<i64> {
    let bits = java_block_bits(palette_len);
    if bits == 0 {
        return Vec::new(); // palette ≤ 1 ⇒ `data` omitted (all palette[0])
    }
    pack_java_packed(indices, bits)
}

/// Pack a Java per-section biome `data` long array (64 cells, index
/// `(y << 4) | (z << 2) | x`).
pub fn pack_java_biome_cells(cells: &[u16], palette_len: usize) -> Vec<i64> {
    let bits = java_biome_bits(palette_len);
    if bits == 0 {
        return Vec::new();
    }
    pack_java_packed(cells, bits)
}

/// Pack one Java `Heightmaps` entry: 256 columns at 9 bits → 37 longs,
/// 7 values per long (LSB-first, non-spanning).
pub fn pack_java_heightmap(values: &[u16]) -> Vec<i64> {
    pack_java_packed(values, 9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_spanning_layout_matches_wiki_vg_example() {
        // bits = 4, values 1..=5: one long holds 16 values; the 5th value
        // must land in the *first* long (no spanning), bits 16..20.
        let packed = [1i64 | (2 << 4) | (3 << 8) | (4 << 12) | (5 << 16), 0];
        let out = unpack_java_packed(&packed, 6, 4);
        assert_eq!(out, [1, 2, 3, 4, 5, 0]);
    }

    #[test]
    fn non_spanning_differs_from_continuous_at_word_edges() {
        // 16 values of 4 bits fit in long 0; value 16 must come from long 1,
        // NOT from long 0's high half (a continuous stream would read 0).
        let packed = [0i64, 7];
        let out = unpack_java_packed(&packed, 17, 4);
        assert_eq!(&out[..16], &[0u16; 16]);
        assert_eq!(out[16], 7);
    }

    #[test]
    fn block_bits_respect_vanilla_minimum() {
        assert_eq!(java_block_bits(1), 0);
        assert_eq!(java_block_bits(2), 4);
        assert_eq!(java_block_bits(16), 4);
        assert_eq!(java_block_bits(17), 5);
        assert_eq!(java_block_bits(4096), 12);
        assert_eq!(java_biome_bits(2), 1);
        assert_eq!(java_biome_bits(5), 3);
    }

    #[test]
    fn biome_cells_unpack_64() {
        let packed = [0b10i64 | (1 << 2)]; // cell0 = 0b10, cell1 = 1 (2 bits each)
        let out = unpack_java_biome_cells(&packed, 3);
        assert_eq!(out[0], 2);
        assert_eq!(out[1], 1);
        assert_eq!(out.len(), 64);
    }

    #[test]
    fn heightmap_unpacks_256_nine_bit_values() {
        // 7 values per long; value 7 sits at bit 0 of long 1.
        let mut packed = [0i64; 37];
        packed[0] = (1 << 0) | (2 << 9) | (3 << 18) | (4 << 27) | (5 << 36) | (6 << 45) | (7 << 54);
        packed[1] = 8;
        let out = unpack_java_heightmap(&packed);
        assert_eq!(out[..9], [1, 2, 3, 4, 5, 6, 7, 8, 0]);
        assert_eq!(out.len(), 256);
    }

    #[test]
    fn heightmap_pack_round_trips_37_longs() {
        // Values exercising the 9-bit ceiling and a cross-long boundary.
        let values: Vec<u16> = (0..256).map(|i| ((i * 13) % 385) as u16).collect();
        let packed = pack_java_heightmap(&values);
        assert_eq!(packed.len(), 37, "256 values at 7/long");
        assert_eq!(unpack_java_heightmap(&packed), values);
    }

    #[test]
    fn pack_is_non_spanning_like_unpack() {
        // 16 values of 4 bits fill long 0 exactly; value 16 must land at
        // bit 0 of long 1 (continuous packing would shift it into long 0's
        // high half).
        // 16 values of 4 bits fill long 0 exactly; the 17th value must land
        // at bit 0 of long 1 (continuous packing would shift it into long
        // 0's high half).
        let mut values: Vec<u16> = (0..16).collect();
        values.push(7);
        let packed = pack_java_packed(&values, 4);
        assert_eq!(packed[0] as u64, 0xfedc_ba98_7654_3210);
        assert_eq!(packed[1], 7);
        assert_eq!(packed.len(), 2);
    }

    #[test]
    fn block_indices_pack_round_trips() {
        let mut indices = vec![0u16; 4096];
        for i in (0..4096).step_by(37) {
            indices[i] = (i % 17) as u16;
        }
        let packed = pack_java_indices(&indices, 17); // 5 bits
        assert_eq!(packed.len(), 4096 / 12 + 1);
        assert_eq!(unpack_java_indices(&packed, 17), indices);
        // Palette ≤ 1 ⇒ no data array at all.
        assert!(pack_java_indices(&indices, 1).is_empty());
        assert!(pack_java_biome_cells(&[0u16; 64], 1).is_empty());
    }

    #[test]
    fn biome_cells_pack_round_trips() {
        let mut cells = [0u16; 64];
        for (i, c) in cells.iter_mut().enumerate() {
            *c = (i % 3) as u16;
        }
        let packed = pack_java_biome_cells(&cells, 3); // 2 bits
        assert_eq!(unpack_java_biome_cells(&packed, 3), cells.to_vec());
    }

    #[test]
    fn permutation_round_trips_and_moves_corners() {
        let mut yzx = vec![0u16; 4096];
        // Java YZX: y-major. Mark every position with a unique tag.
        for y in 0..16usize {
            for z in 0..16usize {
                for x in 0..16usize {
                    yzx[(y << 8) | (z << 4) | x] = (y * 256 + z * 16 + x) as u16;
                }
            }
        }
        let xzy = permute_yzx_to_xzy(&yzx);
        // Bedrock XZY spot checks: (x=1,y=2,z=3) holds tag x*256+z*16+y? No —
        // the tag encodes the ORIGINAL yzx position, so xzy index (1,2,3)
        // must carry the tag of java (y=2,z=3,x=1).
        assert_eq!(xzy[(1 << 8) | (3 << 4) | 2], (2 * 256 + 3 * 16 + 1) as u16);
        // Round trip restores the original.
        let back = permute_xzy_to_yzx(&xzy);
        assert_eq!(back, yzx);
    }
}
