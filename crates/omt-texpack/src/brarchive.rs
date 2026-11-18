//! Reader for the `.brarchive` containers installed Bedrock builds ship
//! their resource-pack files in (`<pack>/__brarchive/<dir>.brarchive`
//! holds the files of `<pack>/<dir>/`).
//!
//! Layout (little-endian), established by inspecting installed packs:
//! * 8-byte magic `7d 27 25 b1 a0 52 70 26`, `u32` entry count, `u32`
//!   version (1);
//! * `count` directory records of 256 bytes: `u8` name length, the name
//!   (UTF-8, zero padded), and at bytes 248..256 the `u32` offset and `u32`
//!   size of the file data;
//! * the file data blob, offsets relative to its start (`16 + 256·count`).
//!
//! Entries are stored uncompressed (raw PNG / TGA / JSON bytes).

use std::collections::HashMap;
use std::path::Path;

pub const MAGIC: [u8; 8] = [0x7d, 0x27, 0x25, 0xb1, 0xa0, 0x52, 0x70, 0x26];
const RECORD: usize = 256;

/// A parsed archive (bytes kept in memory; vanilla archives are small).
#[derive(Debug, Clone)]
pub struct Archive {
    data: Vec<u8>,
    base: usize,
    index: HashMap<String, (usize, usize)>,
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("not a brarchive (bad magic)")]
    Magic,
    #[error("truncated brarchive")]
    Truncated,
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}

impl Archive {
    pub fn parse(data: Vec<u8>) -> Result<Self, ArchiveError> {
        if data.len() < 16 || data[..8] != MAGIC {
            return Err(ArchiveError::Magic);
        }
        let count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        let base = 16usize.checked_add(count.checked_mul(RECORD).ok_or(ArchiveError::Truncated)?).ok_or(ArchiveError::Truncated)?;
        if base > data.len() {
            return Err(ArchiveError::Truncated);
        }
        let mut index = HashMap::with_capacity(count);
        for i in 0..count {
            let r = &data[16 + i * RECORD..16 + (i + 1) * RECORD];
            let len = (r[0] as usize).min(247);
            let name = String::from_utf8_lossy(&r[1..1 + len]).into_owned();
            let off = u32::from_le_bytes([r[248], r[249], r[250], r[251]]) as usize;
            let size = u32::from_le_bytes([r[252], r[253], r[254], r[255]]) as usize;
            if base + off + size > data.len() {
                return Err(ArchiveError::Truncated);
            }
            index.insert(name, (off, size));
        }
        Ok(Self { data, base, index })
    }

    pub fn open(path: &Path) -> Result<Self, ArchiveError> {
        Self::parse(std::fs::read(path)?)
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        let &(off, size) = self.index.get(name)?;
        Some(&self.data[self.base + off..self.base + off + size])
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

/// Build an archive in memory (tests, and to document the layout).
pub fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    let mut off = 0u32;
    for (name, bytes) in files {
        let mut rec = [0u8; RECORD];
        rec[0] = name.len() as u8;
        rec[1..1 + name.len()].copy_from_slice(name.as_bytes());
        rec[248..252].copy_from_slice(&off.to_le_bytes());
        rec[252..256].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&rec);
        off += bytes.len() as u32;
    }
    for (_, bytes) in files {
        out.extend_from_slice(bytes);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_errors() {
        let bytes = build(&[("a.png", b"\x89PNGaaa"), ("b.tga", b"xyz")]);
        let a = Archive::parse(bytes.clone()).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a.get("a.png"), Some(&b"\x89PNGaaa"[..]));
        assert_eq!(a.get("b.tga"), Some(&b"xyz"[..]));
        assert!(a.get("c").is_none());
        assert!(matches!(Archive::parse(b"nope".to_vec()), Err(ArchiveError::Magic)));
        let mut cut = bytes;
        cut.truncate(cut.len() - 2);
        assert!(matches!(Archive::parse(cut), Err(ArchiveError::Truncated)));
    }
}
