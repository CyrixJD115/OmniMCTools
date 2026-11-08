//! Container detection and (de)compression for NBT payloads.
//!
//! `raw` is the naked NBT stream, `gzip` is Java's usual `.dat` wrapping,
//! `zlib` appears in region files (scheme 2) and Bedrock keys.

use std::io::Read;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Raw,
    Gzip,
    Zlib,
}

/// Compression-scheme byte used inside Anvil region entries.
pub const REGION_SCHEME_GZIP: u8 = 1;
pub const REGION_SCHEME_ZLIB: u8 = 2;
pub const REGION_SCHEME_NONE: u8 = 3;

impl Container {
    /// Sniff the wire format. Detection is unambiguous because every valid
    /// magic also happens to be an impossible first NBT tag id.
    pub fn detect(buf: &[u8]) -> Self {
        if buf.len() >= 2 && buf[0] == 0x1f && buf[1] == 0x8b {
            Self::Gzip
        } else if buf.len() >= 2
            && buf[0] == 0x78
            && matches!(buf[1], 0x01 | 0x5e | 0x9c | 0xda)
        {
            Self::Zlib
        } else {
            Self::Raw
        }
    }

    /// Decompress `buf` according to its detected container, returning the
    /// payload and the container that was used.
    pub fn decode(buf: &[u8]) -> Result<(Vec<u8>, Self)> {
        match Self::detect(buf) {
            Self::Raw => Ok((buf.to_vec(), Self::Raw)),
            Self::Gzip => {
                let mut out = Vec::new();
                flate2::read::MultiGzDecoder::new(buf)
                    .read_to_end(&mut out)
                    .map_err(|e| Error::Container(format!("gzip: {e}")))?;
                Ok((out, Self::Gzip))
            }
            Self::Zlib => {
                let mut out = Vec::new();
                flate2::read::ZlibDecoder::new(buf)
                    .read_to_end(&mut out)
                    .map_err(|e| Error::Container(format!("zlib: {e}")))?;
                Ok((out, Self::Zlib))
            }
        }
    }

    pub fn encode(&self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Raw => data.to_vec(),
            Self::Gzip => {
                let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).expect("Vec writer cannot fail");
                e.finish().expect("Vec writer cannot fail")
            }
            Self::Zlib => {
                let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).expect("Vec writer cannot fail");
                e.finish().expect("Vec writer cannot fail")
            }
        }
    }
}

use std::io::Write as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffing() {
        assert_eq!(Container::detect(&[0x1f, 0x8b, 0x08, 0x00]), Container::Gzip);
        assert_eq!(Container::detect(&[0x78, 0x9c, 0x00, 0x00]), Container::Zlib);
        assert_eq!(Container::detect(&[0x0a, 0x00, 0x00]), Container::Raw);
    }

    #[test]
    fn gzip_roundtrip() {
        let data = b"the quick brown fox".repeat(64);
        for c in [Container::Raw, Container::Gzip, Container::Zlib] {
            let enc = c.encode(&data);
            let (dec, used) = Container::decode(&enc).unwrap();
            assert_eq!(used, c);
            assert_eq!(dec, data);
        }
    }
}
