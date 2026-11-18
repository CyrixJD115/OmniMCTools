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
    len: u64,
}

impl RegionFile {
    /// Parse the header of an on-disk `.mca` (lenient mode per UMT). Reads
    /// only the 8 KiB header — chunk payloads are fetched on demand.
    pub fn open(path: &Path) -> Result<Self, RegionError> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        if len < HEADER_BYTES as u64 {
            return Err(RegionError::TooSmall(len));
        }
        let mut header = vec![0u8; HEADER_BYTES];
        file.read_exact(&mut header)?;
        Self::from_header(path, &header, len)
    }

    pub fn from_bytes(path: &Path, bytes: &[u8]) -> Result<Self, RegionError> {
        if bytes.len() < HEADER_BYTES {
            return Err(RegionError::TooSmall(bytes.len() as u64));
        }
        Self::from_header(path, &bytes[..HEADER_BYTES], bytes.len() as u64)
    }

    /// Parse the 8 KiB header of a region file `file_len` bytes long.
    pub fn from_header(path: &Path, bytes: &[u8], file_len: u64) -> Result<Self, RegionError> {
        if bytes.len() < HEADER_BYTES || file_len < HEADER_BYTES as u64 {
            return Err(RegionError::TooSmall(file_len));
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
            let max_sector = file_len.div_ceil(SECTOR_BYTES as u64);
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
        Ok(Self { path: path.to_path_buf(), locations, timestamps, len: file_len })
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
        self.decode_sectors(&bytes[start..end], x, z)
    }

    /// Raw chunk payload read straight from the file: only the chunk's own
    /// sectors are read (single-chunk lookups never load the whole region).
    pub fn read_raw_from_file(&self, x: i32, z: i32) -> Result<Vec<u8>, RegionError> {
        use std::io::{Read, Seek, SeekFrom};
        let Some(loc) = self.location(x, z) else {
            return Err(RegionError::Absent { x, z });
        };
        let start = loc.offset_sectors as u64 * SECTOR_BYTES as u64;
        let end = (start + loc.count_sectors as u64 * SECTOR_BYTES as u64).min(self.len);
        if start + 5 > end {
            return Err(RegionError::Corrupt { x, z, reason: format!("sectors {start}..{end} beyond file {}", self.len) });
        }
        let mut file = std::fs::File::open(&self.path)?;
        file.seek(SeekFrom::Start(start))?;
        let mut buf = vec![0u8; (end - start) as usize];
        file.read_exact(&mut buf)?;
        self.decode_sectors(&buf, x, z)
    }

    /// Parsed chunk NBT read straight from the file (see [`Self::read_raw_from_file`]).
    pub fn read_chunk_from_file(&self, x: i32, z: i32) -> Result<Nbt, RegionError> {
        let raw = self.read_raw_from_file(x, z)?;
        omt_nbt::read(&raw, ByteOrder::Big).map_err(|e| RegionError::Corrupt { x, z, reason: format!("NBT: {e}") })
    }

    fn decode_sectors(&self, sector: &[u8], x: i32, z: i32) -> Result<Vec<u8>, RegionError> {
        if sector.len() < 5 {
            return Err(RegionError::Corrupt { x, z, reason: "truncated sector".into() });
        }
        let len = u32::from_be_bytes([sector[0], sector[1], sector[2], sector[3]]) as usize;
        let scheme = sector[4];
        if len == 0 {
            return Err(RegionError::Corrupt { x, z, reason: "zero length".into() });
        }
        // `len` counts the scheme byte; payload is len-1.
        let payload = &sector[5..5 + (len - 1).min(sector.len() - 5)];
        if scheme & EXTERNAL_FLAG != 0 {
            let ext = self.path.with_file_name(format!("c.{x}.{z}.mcc"));
            let data = std::fs::read(&ext)
                .map_err(|e| RegionError::Corrupt { x, z, reason: format!("external payload {}: {e}", ext.display()) })?;
            return match scheme & !EXTERNAL_FLAG {
                REGION_SCHEME_GZIP | REGION_SCHEME_ZLIB => Container::decode(&data)
                    .map(|(d, _)| d)
                    .map_err(|e| RegionError::Corrupt { x, z, reason: e.to_string() }),
                REGION_SCHEME_NONE => Ok(data),
                other => Err(RegionError::Corrupt { x, z, reason: format!("unknown scheme {other}") }),
            };
        }
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

/// Outcome of [`RegionFile::retain`].
#[derive(Debug, Default)]
pub struct Retained {
    /// The rebuilt region image, or `None` when no chunk survived (the
    /// caller deletes the file — vanilla never keeps empty regions).
    pub image: Option<Vec<u8>>,
    /// Absolute coords of chunks that were dropped.
    pub removed: Vec<(i32, i32)>,
    /// Dropped chunks whose payload lived in an external `c.x.z.mcc` file
    /// (scheme high bit set); the caller deletes those files.
    pub removed_external: Vec<(i32, i32)>,
}

impl RegionFile {
    /// Rebuild this region keeping only chunks where `keep(x, z)` is true.
    ///
    /// Kept chunks are copied **sector-verbatim** (length, scheme byte,
    /// compressed payload, timestamp) and packed contiguously, so untouched
    /// chunks stay byte-identical while dead sectors are reclaimed.
    pub fn retain(
        &self,
        bytes: &[u8],
        rx: i32,
        rz: i32,
        keep: impl Fn(i32, i32) -> bool,
    ) -> Retained {
        let mut out = Retained::default();
        let mut offsets = [0u8; 4096];
        let mut times = [0u8; 4096];
        let mut body: Vec<u8> = Vec::new();
        for (&idx, loc) in &self.locations {
            let x = rx * 32 + (idx % 32) as i32;
            let z = rz * 32 + (idx / 32) as i32;
            let start = loc.offset_sectors as usize * SECTOR_BYTES;
            let end = (start + loc.count_sectors as usize * SECTOR_BYTES).min(bytes.len());
            let external = bytes.get(start + 4).is_some_and(|s| s & 0x80 != 0);
            if !keep(x, z) {
                out.removed.push((x, z));
                if external {
                    out.removed_external.push((x, z));
                }
                continue;
            }
            if start >= end {
                continue; // lenient: unreadable entry is dropped, like the reader
            }
            let sectors = (end - start).div_ceil(SECTOR_BYTES);
            let offset = (HEADER_BYTES + body.len()) / SECTOR_BYTES;
            let i = idx as usize * 4;
            offsets[i] = (offset >> 16) as u8;
            offsets[i + 1] = (offset >> 8) as u8;
            offsets[i + 2] = offset as u8;
            offsets[i + 3] = sectors as u8;
            if let Some(t) = self.timestamps.get(&idx) {
                times[i..i + 4].copy_from_slice(&t.to_be_bytes());
            }
            body.extend_from_slice(&bytes[start..end]);
            body.resize(body.len().div_ceil(SECTOR_BYTES) * SECTOR_BYTES, 0);
        }
        if body.is_empty() {
            return out;
        }
        let mut image = Vec::with_capacity(HEADER_BYTES + body.len());
        image.extend_from_slice(&offsets);
        image.extend_from_slice(&times);
        image.extend_from_slice(&body);
        out.image = Some(image);
        out
    }
}

impl RegionFile {
    /// Rebuild this region with chunk `(x, z)` replaced by (or set to) the
    /// uncompressed NBT payload `raw` (zlib, current timestamp). Every other
    /// chunk is copied sector-verbatim. Used by the NBT editor to save an
    /// edited chunk back into its region.
    pub fn with_chunk(&self, bytes: &[u8], x: i32, z: i32, raw: &[u8], timestamp: u32) -> Vec<u8> {
        let target = local_index(x, z);
        let mut offsets = [0u8; 4096];
        let mut times = [0u8; 4096];
        let mut body: Vec<u8> = Vec::new();
        let mut place = |idx: u16, entry: &[u8], ts: Option<u32>, body: &mut Vec<u8>| {
            let sectors = entry.len().div_ceil(SECTOR_BYTES).max(1);
            let offset = (HEADER_BYTES + body.len()) / SECTOR_BYTES;
            let i = idx as usize * 4;
            offsets[i] = (offset >> 16) as u8;
            offsets[i + 1] = (offset >> 8) as u8;
            offsets[i + 2] = offset as u8;
            offsets[i + 3] = sectors as u8;
            if let Some(t) = ts {
                times[i..i + 4].copy_from_slice(&t.to_be_bytes());
            }
            body.extend_from_slice(entry);
            body.resize(body.len().div_ceil(SECTOR_BYTES) * SECTOR_BYTES, 0);
        };
        let mut fresh = Vec::new();
        let compressed = Container::Zlib.encode(raw);
        if (compressed.len() + 5).div_ceil(SECTOR_BYTES) > MAX_INLINE_SECTORS {
            // Vanilla's oversized form: stub + external c.x.z.mcc.
            if let Some(dir) = self.path.parent() {
                let _ = std::fs::write(dir.join(format!("c.{x}.{z}.mcc")), &compressed);
            }
            fresh.extend_from_slice(&1u32.to_be_bytes());
            fresh.push(REGION_SCHEME_ZLIB | EXTERNAL_FLAG);
        } else {
            fresh.extend_from_slice(&(compressed.len() as u32 + 1).to_be_bytes());
            fresh.push(REGION_SCHEME_ZLIB);
            fresh.extend_from_slice(&compressed);
        }
        let mut wrote_target = false;
        for (&idx, loc) in &self.locations {
            if idx == target {
                place(idx, &fresh, Some(timestamp), &mut body);
                wrote_target = true;
                continue;
            }
            let start = loc.offset_sectors as usize * SECTOR_BYTES;
            let end = (start + loc.count_sectors as usize * SECTOR_BYTES).min(bytes.len());
            if start < end {
                place(idx, &bytes[start..end], self.timestamps.get(&idx).copied(), &mut body);
            }
        }
        if !wrote_target {
            place(target, &fresh, Some(timestamp), &mut body);
        }
        let mut image = Vec::with_capacity(HEADER_BYTES + body.len());
        image.extend_from_slice(&offsets);
        image.extend_from_slice(&times);
        image.extend_from_slice(&body);
        image
    }
}

fn local_index(x: i32, z: i32) -> u16 {
    (((x & 31) + (z & 31) * 32) & 0x3ff) as u16
}

/// Build a region file image from chunk payloads (already-NBT-encoded bytes).
/// Sector allocation is first-fit; the output is a valid `.mca`.
/// One pending chunk: absolute coords, raw payload, timestamp.
type PendingChunk = ((i32, i32), Vec<u8>, u32);

pub struct RegionWriter {
    /// local index → pending chunk
    entries: BTreeMap<u16, PendingChunk>,
}

/// Largest sector count a location entry can express (one byte).
pub const MAX_INLINE_SECTORS: usize = 255;
/// Scheme-byte flag: the payload lives in `c.<x>.<z>.mcc` next to the region.
pub const EXTERNAL_FLAG: u8 = 0x80;

/// A finished region image plus any oversized chunks that must be written
/// as external `c.<x>.<z>.mcc` files (compressed payload only).
#[derive(Debug, Default)]
pub struct RegionImage {
    pub region: Vec<u8>,
    pub externals: Vec<((i32, i32), Vec<u8>)>,
}

impl RegionImage {
    /// Write the region and its external payloads into `dir` atomically
    /// (temp + rename per file).
    pub fn write_to(&self, dir: &Path, rx: i32, rz: i32) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        for ((x, z), data) in &self.externals {
            let p = dir.join(format!("c.{x}.{z}.mcc"));
            let tmp = p.with_extension("mcc.omt_tmp");
            std::fs::write(&tmp, data)?;
            std::fs::rename(&tmp, &p)?;
        }
        let p = dir.join(format!("r.{rx}.{rz}.mca"));
        let tmp = p.with_extension("mca.omt_tmp");
        std::fs::write(&tmp, &self.region)?;
        std::fs::rename(&tmp, &p)
    }
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
        self.entries.insert(local_index(x, z), ((x, z), raw, timestamp));
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialize with zlib-compressed payloads (current vanilla default).
    /// Serialize with zlib payloads (vanilla default). Only for regions
    /// whose chunks all fit inline (< 1 MiB compressed); use
    /// [`Self::finish_image`] when a chunk may be oversized.
    pub fn finish(&self) -> Vec<u8> {
        let image = self.finish_image();
        debug_assert!(image.externals.is_empty(), "oversized chunk needs finish_image()");
        image.region
    }

    /// Serialize, moving chunks that exceed 255 sectors to external
    /// payloads exactly as vanilla does (scheme byte `2 | 0x80`, a 1-sector
    /// stub in the region, data in `c.<x>.<z>.mcc`).
    pub fn finish_image(&self) -> RegionImage {
        let mut offsets = [0u8; 4096];
        let mut times = [0u8; 4096];
        let mut body: Vec<u8> = Vec::new();
        let mut externals = Vec::new();
        for (&idx, (xz, raw, ts)) in &self.entries {
            let compressed = Container::Zlib.encode(raw);
            // 4-byte length + 1 scheme byte + payload: the length prefix
            // counts toward the sectors (it was once left out, truncating
            // chunks whose payload ended within 4 bytes of a boundary).
            let mut entry = Vec::with_capacity(compressed.len() + 5);
            if (compressed.len() + 5).div_ceil(SECTOR_BYTES) > MAX_INLINE_SECTORS {
                entry.extend_from_slice(&1u32.to_be_bytes());
                entry.push(REGION_SCHEME_ZLIB | EXTERNAL_FLAG);
                externals.push((*xz, compressed));
            } else {
                entry.extend_from_slice(&(compressed.len() as u32 + 1).to_be_bytes());
                entry.push(REGION_SCHEME_ZLIB);
                entry.extend_from_slice(&compressed);
            }
            let sectors = entry.len().div_ceil(SECTOR_BYTES).max(1);
            entry.resize(sectors * SECTOR_BYTES, 0);
            let offset = (HEADER_BYTES + body.len()) / SECTOR_BYTES;
            let i = idx as usize * 4;
            offsets[i] = (offset >> 16) as u8;
            offsets[i + 1] = (offset >> 8) as u8;
            offsets[i + 2] = offset as u8;
            offsets[i + 3] = sectors as u8;
            times[i..i + 4].copy_from_slice(&ts.to_be_bytes());
            body.extend_from_slice(&entry);
        }
        let mut region = Vec::with_capacity(HEADER_BYTES + body.len());
        region.extend_from_slice(&offsets);
        region.extend_from_slice(&times);
        region.extend_from_slice(&body);
        RegionImage { region, externals }
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

    /// Incompressible payload of exactly `n` compressed bytes is hard to
    /// target, so sweep sizes around the boundary and require every chunk
    /// to read back — the old writer truncated some of them.
    #[test]
    fn sector_boundary_and_external_payloads() {
        let dir = std::env::temp_dir().join(format!("omt-region-ext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut seed = 12345u64;
        let mut noise = |n: usize| -> Vec<u8> {
            (0..n)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect()
        };
        let mut w = RegionWriter::new();
        let mut expect = Vec::new();
        for (i, n) in (4060..4110).enumerate() {
            let raw = noise(n);
            let (x, z) = ((i % 32) as i32, (i / 32) as i32);
            w.put(x, z, raw.clone(), 1);
            expect.push(((x, z), raw));
        }
        let huge = noise(1_200_000); // > 255 sectors after (non-)compression
        w.put(5, 9, huge.clone(), 1);
        expect.push(((5, 9), huge));
        let image = w.finish_image();
        assert_eq!(image.externals.len(), 1);
        image.write_to(&dir, 0, 0).unwrap();
        let path = dir.join("r.0.0.mca");
        let bytes = std::fs::read(&path).unwrap();
        let region = RegionFile::from_bytes(&path, &bytes).unwrap();
        for ((x, z), raw) in &expect {
            assert_eq!(&region.read_raw(&bytes, *x, *z).unwrap(), raw, "chunk {x},{z}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn retain_and_with_chunk_keep_others_verbatim() {
        let mut w = RegionWriter::new();
        for (x, z) in [(0, 0), (1, 0), (5, 7), (31, 31)] {
            w.put(x, z, demo_chunk_nbt(x, z), 7);
        }
        let bytes = w.finish();
        let path = Path::new("r.0.0.mca");
        let region = RegionFile::from_bytes(path, &bytes).unwrap();

        let kept = region.retain(&bytes, 0, 0, |x, _| x != 1);
        assert_eq!(kept.removed, vec![(1, 0)]);
        let image = kept.image.unwrap();
        let r2 = RegionFile::from_bytes(path, &image).unwrap();
        assert_eq!(r2.chunk_count(), 3);
        for (x, z) in [(0, 0), (5, 7), (31, 31)] {
            assert_eq!(r2.read_raw(&image, x, z).unwrap(), region.read_raw(&bytes, x, z).unwrap());
            assert_eq!(r2.timestamp(x, z), Some(7));
        }
        assert!(region.retain(&bytes, 0, 0, |_, _| false).image.is_none(), "empty region → delete");

        // Replace one chunk, add another; the rest stay byte-identical.
        let edited = demo_chunk_nbt(99, 99);
        let img = region.with_chunk(&bytes, 5, 7, &edited, 42);
        let r3 = RegionFile::from_bytes(path, &img).unwrap();
        assert_eq!(r3.read_raw(&img, 5, 7).unwrap(), edited);
        assert_eq!(r3.timestamp(5, 7), Some(42));
        assert_eq!(r3.read_raw(&img, 0, 0).unwrap(), region.read_raw(&bytes, 0, 0).unwrap());
        let img2 = r3.with_chunk(&img, 2, 2, &edited, 1);
        assert_eq!(RegionFile::from_bytes(path, &img2).unwrap().chunk_count(), 5);
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
