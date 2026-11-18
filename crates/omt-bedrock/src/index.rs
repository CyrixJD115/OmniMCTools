//! [`ColumnIndex`]: which chunk columns exist and which of their records
//! (subchunk Y levels, version, data3D, …) are present — built from ONE
//! streaming, table-parallel **key-only** scan of the database.
//!
//! This replaces whole-database snapshots for bulk readers. Memory is
//! bounded by the number of 32×32-chunk regions (~5 KB each), never by the
//! world's data size; map tiles then fetch exactly the keys that exist with
//! batched point reads (no wasted probes for absent subchunks).

use std::collections::HashMap;

use omt_world::ChunkSet;

use crate::keys::{self, tag, DbKey};

/// Lowest subchunk Y tracked (bit 0 of the mask).
pub const SUBCHUNK_MIN_Y: i8 = -4;
/// Highest subchunk Y tracked (bit 31).
pub const SUBCHUNK_MAX_Y: i8 = 27;

/// Record-presence flags of a column.
pub mod flag {
    pub const VERSION: u8 = 1;
    pub const DATA_3D: u8 = 2;
    pub const FINALIZED: u8 = 4;
    pub const BLOCK_ENTITIES: u8 = 8;
    pub const DATA_2D_LEGACY: u8 = 16;
    pub const DIGP: u8 = 32;
    pub const OTHER: u8 = 64;
}

/// Presence summary of one chunk column.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColumnInfo {
    /// Bit `y - SUBCHUNK_MIN_Y` set ⇔ subchunk `y` exists.
    pub subchunks: u32,
    pub flags: u8,
}

impl ColumnInfo {
    /// The column counts as a chunk (same rule as `list_chunks`): any
    /// subchunk, or a version / data3D / finalized-state record.
    pub fn is_chunk(&self) -> bool {
        self.subchunks != 0 || self.flags & (flag::VERSION | flag::DATA_3D | flag::FINALIZED) != 0
    }

    pub fn has_subchunk(&self, y: i8) -> bool {
        (SUBCHUNK_MIN_Y..=SUBCHUNK_MAX_Y).contains(&y) && self.subchunks & (1 << (y - SUBCHUNK_MIN_Y)) != 0
    }

    /// Highest present subchunk Y.
    pub fn top_subchunk(&self) -> Option<i8> {
        (self.subchunks != 0).then(|| (31 - self.subchunks.leading_zeros()) as i8 + SUBCHUNK_MIN_Y)
    }

    fn merge(&mut self, other: ColumnInfo) {
        self.subchunks |= other.subchunks;
        self.flags |= other.flags;
    }

    /// Exact keys the map renderer needs for this column (block data +
    /// heightmap/biomes; no block entities or actors).
    pub fn render_keys(&self, x: i32, z: i32, dim: u8) -> Vec<Vec<u8>> {
        let mut out = Vec::with_capacity(self.subchunks.count_ones() as usize + 3);
        if self.flags & flag::VERSION != 0 {
            out.push(keys::version_key(x, z, dim));
        }
        if self.flags & flag::DATA_3D != 0 {
            out.push(keys::data3d_key(x, z, dim));
        }
        if self.flags & flag::FINALIZED != 0 {
            out.push(keys::chunk_record(x, z, dim, tag::FINALIZED_STATE));
        }
        let mut bits = self.subchunks;
        while bits != 0 {
            let b = bits.trailing_zeros() as i8;
            bits &= bits - 1;
            let y = b + SUBCHUNK_MIN_Y;
            // `decode_column` reads -4..=19 only.
            if y <= 19 {
                out.push(keys::subchunk_key(x, z, dim, y));
            }
        }
        out
    }
}

const REGION: i32 = 32;

#[derive(Debug, Clone)]
struct Region {
    cols: Box<[ColumnInfo; 1024]>,
}

impl Default for Region {
    fn default() -> Self {
        Self { cols: Box::new([ColumnInfo::default(); 1024]) }
    }
}

/// Column presence of one dimension.
#[derive(Debug, Clone, Default)]
pub struct DimIndex {
    regions: HashMap<(i32, i32), Region>,
    chunks: ChunkSet,
}

#[inline]
fn split(cx: i32, cz: i32) -> ((i32, i32), usize) {
    (
        (cx.div_euclid(REGION), cz.div_euclid(REGION)),
        (cz.rem_euclid(REGION) * REGION + cx.rem_euclid(REGION)) as usize,
    )
}

impl DimIndex {
    fn add(&mut self, cx: i32, cz: i32, info: ColumnInfo) {
        let (r, i) = split(cx, cz);
        self.regions.entry(r).or_default().cols[i].merge(info);
    }

    fn finish(&mut self) {
        let mut set = ChunkSet::new();
        for (&(rx, rz), region) in &self.regions {
            for (i, c) in region.cols.iter().enumerate() {
                if c.is_chunk() {
                    set.insert(rx * REGION + i as i32 % REGION, rz * REGION + i as i32 / REGION);
                }
            }
        }
        self.chunks = set;
    }

    /// Info of a column that exists as a chunk.
    pub fn get(&self, cx: i32, cz: i32) -> Option<ColumnInfo> {
        let (r, i) = split(cx, cz);
        let c = self.regions.get(&r)?.cols[i];
        c.is_chunk().then_some(c)
    }

    /// Info including record-only columns (e.g. a lone `digp`).
    pub fn get_any(&self, cx: i32, cz: i32) -> Option<ColumnInfo> {
        let (r, i) = split(cx, cz);
        let c = self.regions.get(&r)?.cols[i];
        (c != ColumnInfo::default()).then_some(c)
    }

    /// The set of chunks (columns satisfying [`ColumnInfo::is_chunk`]).
    pub fn chunks(&self) -> &ChunkSet {
        &self.chunks
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Approximate heap bytes held.
    pub fn heap_bytes(&self) -> usize {
        self.regions.len() * (std::mem::size_of::<[ColumnInfo; 1024]>() + 48) + self.chunks.heap_bytes()
    }
}

/// Column presence of every dimension in the database.
#[derive(Debug, Clone, Default)]
pub struct ColumnIndex {
    dims: HashMap<u8, DimIndex>,
    /// Keys visited by the scan (diagnostics).
    pub keys_scanned: u64,
}

impl ColumnIndex {
    /// Fold one raw key into a partial index (scan visitor).
    pub fn visit_key(partial: &mut HashMap<(u8, i32, i32), ColumnInfo>, key: &[u8]) {
        let (dim, x, z, info) = match keys::decode_key(key) {
            DbKey::Subchunk { x, z, dim, y } => {
                if !(SUBCHUNK_MIN_Y..=SUBCHUNK_MAX_Y).contains(&y) {
                    return;
                }
                (dim, x, z, ColumnInfo { subchunks: 1 << (y - SUBCHUNK_MIN_Y), flags: 0 })
            }
            DbKey::ChunkRecord { x, z, dim, tag: t } => {
                let f = match t {
                    tag::VERSION => flag::VERSION,
                    tag::DATA_3D => flag::DATA_3D,
                    tag::FINALIZED_STATE => flag::FINALIZED,
                    tag::BLOCK_ENTITIES => flag::BLOCK_ENTITIES,
                    tag::DATA_2D_LEGACY => flag::DATA_2D_LEGACY,
                    _ => flag::OTHER,
                };
                (dim, x, z, ColumnInfo { subchunks: 0, flags: f })
            }
            DbKey::Digp { x, z, dim } => (dim, x, z, ColumnInfo { subchunks: 0, flags: flag::DIGP }),
            _ => return,
        };
        partial.entry((dim, x, z)).or_default().merge(info);
    }

    /// Merge scan partitions into the final index.
    pub fn from_partials(parts: Vec<HashMap<(u8, i32, i32), ColumnInfo>>, keys_scanned: u64) -> Self {
        let mut dims: HashMap<u8, DimIndex> = HashMap::new();
        for part in parts {
            for ((d, x, z), info) in part {
                dims.entry(d).or_default().add(x, z, info);
            }
        }
        for d in dims.values_mut() {
            d.finish();
        }
        Self { dims, keys_scanned }
    }

    pub fn dim(&self, dim: u8) -> Option<&DimIndex> {
        self.dims.get(&dim)
    }

    /// Dimensions present (sorted).
    pub fn dims(&self) -> Vec<u8> {
        let mut v: Vec<u8> = self.dims.keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// Take one dimension out (callers that only need one keep less).
    pub fn into_dim(mut self, dim: u8) -> DimIndex {
        self.dims.remove(&dim).unwrap_or_default()
    }

    pub fn heap_bytes(&self) -> usize {
        self.dims.values().map(DimIndex::heap_bytes).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_fold_into_column_info() {
        let mut part = HashMap::new();
        for k in [
            keys::subchunk_key(3, -2, 0, -4),
            keys::subchunk_key(3, -2, 0, 5),
            keys::version_key(3, -2, 0),
            keys::data3d_key(3, -2, 0),
            keys::subchunk_key(3, -2, 1, 2),
            keys::digp_key(9, 9, 0),
        ] {
            ColumnIndex::visit_key(&mut part, &k);
        }
        let idx = ColumnIndex::from_partials(vec![part], 6);
        let ow = idx.dim(0).unwrap();
        let c = ow.get(3, -2).unwrap();
        assert!(c.has_subchunk(-4) && c.has_subchunk(5) && !c.has_subchunk(4));
        assert_eq!(c.top_subchunk(), Some(5));
        assert_eq!(c.flags, flag::VERSION | flag::DATA_3D);
        assert_eq!(c.render_keys(3, -2, 0).len(), 4);
        // A lone digp is not a chunk.
        assert!(ow.get(9, 9).is_none() && ow.get_any(9, 9).is_some());
        assert_eq!(ow.len(), 1);
        assert_eq!(idx.dim(1).unwrap().len(), 1);
        assert_eq!(idx.dims(), vec![0, 1]);
    }
}
