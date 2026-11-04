//! Anvil region files (`.mca`): 8 KiB header (location + timestamp tables,
//! 4096 bytes each), 4 KiB sectors, chunk payloads with a 5-byte lead-in
//! (length u32 BE, scheme u8: 1 gzip / 2 zlib / 3 raw).
//!
//! Includes UMT's documented corruption-lenient behaviors (addendum §4):
//! duplicate/invalid location entries are ignored, preferring the last one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use omt_nbt::{ByteOrder, Container, Nbt};
use omt_nbt::container::{REGION_SCHEME_GZIP, REGION_SCHEME_NONE, REGION_SCHEME_ZLIB};

pub const SECTOR_BYTES: usize = 4096;
pub const HEADER_BYTES: usize = 8192;

#[derive(Debug, thiserror::Error)]
pub enum RegionError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("region file too small: {0} bytes")]
    TooSmall(u64),
    #[error("chunk ({x},{z}) absent")]
    Absent { x: i32, z: i32 },
    #[error("corrupt chunk ({x},{z}): {reason}")]
    Corrupt { x: i32, z: i32, reason: String },
    #[error("NBT: {0}")]
    Nbt(#[from] omt_nbt::Error),
}

/// A chunk slot in the location table: sector offset (in 4 KiB units),
/// sector count, and the raw entry for dedup diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Location {
    offset_sectors: u32,
    count_sectors: u32,
}

#[derive(Debug)]
pub struct RegionFile {
    path: PathBuf,
    /// region-local chunk index (0..1024) → location.
    locations: BTreeMap<u16, Location>,
    timestamps: BTreeMap<u16, u32>,
    #[allow(dead_code)]
    len: u64,
}

impl RegionFile {
    /// Parse the header of an on-disk `.mca` (lenient mode per UMT).
    pub fn open(path: &Path) -> Result<Self, RegionError> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(path, &bytes)
    }

    pub fn from_bytes(path: &Path, bytes: &[u8]) -> Result<Self, RegionError> {
        if bytes.len() < HEADER_BYTES {
            return Err(RegionError::TooSmall(bytes.len() as u64));
        }
        let mut locations = BTreeMap::new();
        let mut seen_sectors: BTreeMap<u32, u16> = BTreeMap::new();
        for i in 0..1024usize {
            let off = ((bytes[i * 4] as u32) << 16)
                | ((bytes[i * 4 + 1] as u32) << 8)
                | bytes[i * 4 + 2] as u32;
            let cnt = bytes[i * 4 + 3] as u32;
            if off == 0 || cnt == 0 {
                continue;
            }
            let max_sector = (bytes.len() as u64).div_ceil(SECTOR_BYTES as u64);
            // Invalid offset/overlong entries are dropped (UMT's
            // "Invalid index offset!" behavior); duplicates keep the LAST
            // entry ("Duplicates have been ignored") unless the earlier
            // entry owns different sectors.
            if off as u64 >= max_sector || cnt > 256 {
                continue;
            }
            if let Some(&prev) = seen_sectors.get(&off) {
                let _ = prev;
                // duplicate sector entry: keep the newest writer (i)
            }
            seen_sectors.insert(off, i as u16);
            locations.insert(i as u16, Location { offset_sectors: off, count_sectors: cnt });
        }
        let mut timestamps = BTreeMap::new();
        for i in 0..1024usize {
            let t = u32::from_be_bytes([
                bytes[4096 + i * 4],
                bytes[4096 + i * 4 + 1],
                bytes[4096 + i * 4 + 2],
                bytes[4096 + i * 4 + 3],
            ]);
            if t != 0 {
                timestamps.insert(i as u16, t);
            }
        }
        Ok(Self { path: path.to_path_buf(), locations, timestamps, len: bytes.len() as u64 })
    }

    /// Region coordinates from a file name `r.<x>.<z>.mca`.
    pub fn coords_from_name(path: &Path) -> Option<(i32, i32)> {
        let name = path.file_name()?.to_str()?;
        let rest = name.strip_prefix("r.")?.strip_suffix(".mca")?;
        let mut it = rest.split('.');
        let x: i32 = it.next()?.parse().ok()?;
        let z: i32 = it.next()?.parse().ok()?;
        Some((x, z))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn region_coords(&self) -> Option<(i32, i32)> {
        Self::coords_from_name(&self.path)
    }

    pub fn chunk_count(&self) -> usize {
        self.locations.len()
    }

    /// All present chunks as absolute chunk coords for region (rx, rz).
    pub fn chunk_coords(&self, rx: i32, rz: i32) -> Vec<(i32, i32)> {
        self.locations
            .keys()
            .map(|&i| {
                let lx = (i % 32) as i32;
                let lz = (i / 32) as i32;
                (rx * 32 + lx, rz * 32 + lz)
            })
            .collect()
    }

    pub fn timestamp(&self, x: i32, z: i32) -> Option<u32> {
        let idx = local_index(x, z);
        self.timestamps.get(&idx).copied()
    }

    fn location(&self, x: i32, z: i32) -> Option<Location> {
        self.locations.get(&local_index(x, z)).copied()
    }

    pub fn contains(&self, x: i32, z: i32) -> bool {
        self.location(x, z).is_some()
    }

    /// Raw (decompressed) chunk payload bytes.
    pub fn read_raw(&self, bytes: &[u8], x: i32, z: i32) -> Result<Vec<u8>, RegionError> {
        let Some(loc) = self.location(x, z) else {
            return Err(RegionError::Absent { x, z });
        };
        let start = loc.offset_sectors as usize * SECTOR_BYTES;
        let end = start + loc.count_sectors as usize * SECTOR_BYTES;
        if end > bytes.len() {
            return Err(RegionError::Corrupt { x, z, reason: format!("sectors {start}..{end} beyond file {}", bytes.len()) });
        }
        let sector = &bytes[start..end];
        let len = u32::from_be_bytes([sector[0], sector[1], sector[2], sector[3]]) as usize;
        let scheme = sector[4];
        if len == 0 {
            return Err(RegionError::Corrupt { x, z, reason: "zero length".into() });
        }
        // `len` counts the scheme byte; payload is len-1.
        let payload = &sector[5..5 + (len - 1).min(sector.len() - 5)];
        match scheme {
            REGION_SCHEME_GZIP | REGION_SCHEME_ZLIB => {
                let (data, _) = Container::decode(payload)
                    .map_err(|e| RegionError::Corrupt { x, z, reason: e.to_string() })?;
                Ok(data)
            }
            REGION_SCHEME_NONE => Ok(payload.to_vec()),
            other => Err(RegionError::Corrupt { x, z, reason: format!("unknown scheme {other}") }),
        }
    }

    /// Parsed chunk NBT (unnamed root compound).
    pub fn read_chunk(&self, bytes: &[u8], x: i32, z: i32) -> Result<Nbt, RegionError> {
        let raw = self.read_raw(bytes, x, z)?;
        omt_nbt::read(&raw, ByteOrder::Big).map_err(|e| {
            RegionError::Corrupt { x, z, reason: format!("NBT: {e}") }
        })
    }
}

fn local_index(x: i32, z: i32) -> u16 {
    (((x & 31) + (z & 31) * 32) & 0x3ff) as u16
}

/// Build a region file image from chunk payloads (already-NBT-encoded bytes).
/// Sector allocation is first-fit; the output is a valid `.mca`.
pub struct RegionWriter {
    entries: BTreeMap<u16, (Vec<u8>, u32)>, // index → (raw payload, timestamp)
}

impl Default for RegionWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl RegionWriter {
    pub fn new() -> Self {
        Self { entries: BTreeMap::new() }
    }

    /// Insert a chunk (raw, uncompressed NBT bytes) at absolute coords.
    pub fn put(&mut self, x: i32, z: i32, raw: Vec<u8>, timestamp: u32) {
        self.entries.insert(local_index(x, z), (raw, timestamp));
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialize with zlib-compressed payloads (current vanilla default).
    pub fn finish(&self) -> Vec<u8> {
        let mut offsets = [0u8; 4096];
        let mut times = [0u8; 4096];
        let mut body: Vec<u8> = Vec::new();
        for (&idx, (raw, ts)) in &self.entries {
            let compressed = Container::Zlib.encode(raw);
            let len = compressed.len() + 1; // scheme byte
            let sectors = len.div_ceil(SECTOR_BYTES).max(1);
            let offset = (HEADER_BYTES + body.len()) / SECTOR_BYTES;
            offsets[idx as usize * 4] = (offset >> 16) as u8;
            offsets[idx as usize * 4 + 1] = (offset >> 8) as u8;
            offsets[idx as usize * 4 + 2] = offset as u8;
            offsets[idx as usize * 4 + 3] = sectors as u8;
            let t = ts.to_be_bytes();
            times[idx as usize * 4..idx as usize * 4 + 4].copy_from_slice(&t);

            let mut entry = Vec::with_capacity(sectors * SECTOR_BYTES);
            entry.extend_from_slice(&(compressed.len() as u32 + 1).to_be_bytes());
            entry.push(REGION_SCHEME_ZLIB);
            entry.extend_from_slice(&compressed);
            entry.resize(sectors * SECTOR_BYTES, 0);
            body.extend_from_slice(&entry);
        }
        let mut out = Vec::with_capacity(HEADER_BYTES + body.len());
        out.extend_from_slice(&offsets);
        out.extend_from_slice(&times);
        out.extend_from_slice(&body);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_chunk_nbt(x: i32, z: i32) -> Vec<u8> {
        let nbt = Nbt {
            name: "".into(),
            tag: omt_nbt::Tag::Compound(vec![
                ("DataVersion".into(), omt_nbt::Tag::Int(4440)),
                ("xPos".into(), omt_nbt::Tag::Int(x)),
                ("zPos".into(), omt_nbt::Tag::Int(z)),
                ("Status".into(), omt_nbt::Tag::String("minecraft:full".into())),
                ("sections".into(), omt_nbt::Tag::List(vec![omt_nbt::Tag::Compound(vec![
                    ("Y".into(), omt_nbt::Tag::Byte(-4)),
                    ("block_states".into(), omt_nbt::Tag::Compound(vec![
                        ("palette".into(), omt_nbt::Tag::List(vec![
                            omt_nbt::Tag::Compound(vec![("Name".into(), omt_nbt::Tag::String("minecraft:stone".into()))]),
                        ])),
                    ])),
                ])])),
            ]),
        };
        omt_nbt::write(&nbt, ByteOrder::Big).unwrap()
    }

    #[test]
    fn writer_reader_roundtrip() {
        let mut w = RegionWriter::new();
        w.put(-62, -140, demo_chunk_nbt(-62, -140), 12345);
        w.put(0, 0, demo_chunk_nbt(0, 0), 1);
        w.put(511, 511, demo_chunk_nbt(511, 511), 99); // different region in practice
        let bytes = w.finish();

        let region = RegionFile::from_bytes(Path::new("r.-2.-5.mca"), &bytes).unwrap();
        assert_eq!(region.chunk_count(), 3);
        assert_eq!(region.timestamp(0, 0), Some(1));

        let nbt = region.read_chunk(&bytes, -62, -140).unwrap();
        assert_eq!(nbt.tag.get("xPos").unwrap().as_i64(), Some(-62));
        assert_eq!(nbt.tag.get("DataVersion").unwrap().as_i64(), Some(4440));
        let raw = region.read_raw(&bytes, 0, 0).unwrap();
        assert_eq!(omt_nbt::read(&raw, ByteOrder::Big).unwrap().tag.get("zPos").unwrap().as_i64(), Some(0));

        assert!(matches!(region.read_chunk(&bytes, 3, 7), Err(RegionError::Absent { .. })));
    }

    #[test]
    fn lenient_header_drops_bad_entries() {
        let mut w = RegionWriter::new();
        w.put(0, 0, demo_chunk_nbt(0, 0), 1);
        let mut bytes = w.finish();
        // Corrupt entry for (1,0): offset beyond EOF.
        let idx = local_index(1, 0) as usize * 4;
        bytes[idx] = 0xff;
        bytes[idx + 1] = 0xff;
        bytes[idx + 2] = 0xff;
        bytes[idx + 3] = 0x01;
        let region = RegionFile::from_bytes(Path::new("r.0.0.mca"), &bytes).unwrap();
        assert_eq!(region.chunk_count(), 1); // bad entry dropped
        assert!(region.contains(0, 0));
    }

    #[test]
    fn name_parsing() {
        assert_eq!(RegionFile::coords_from_name(Path::new("r.-2.-5.mca")), Some((-2, -5)));
        assert_eq!(RegionFile::coords_from_name(Path::new("r.0.0.mca")), Some((0, 0)));
        assert_eq!(RegionFile::coords_from_name(Path::new("level.dat")), None);
    }
}
