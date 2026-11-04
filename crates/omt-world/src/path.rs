//! The virtual path namespace — UMT's best design idea (addendum §5): one
//! addressing scheme over every backing store, which is what lets the Lua
//! extension API and every module treat Java/Bedrock identically.
//!
//! Grammar (from UMT strings):
//! ```text
//! :DB/:CHUNK/<dim>/<x>/<z>/<tag>   chunk record inside the LevelDB
//! :DB/<raw-key-hex>                arbitrary database key
//! /region/r.<x>.<z>.mca            Java files, as-is
//! /entities/r.<x>.<z>.mca
//! /DIM-1/... /DIM1/... /dimensions/...
//! level.dat, levelname.txt, world_icon.jpeg
//! ```

use std::fmt;
use std::str::FromStr;

use crate::store::WorldNode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VPath {
    /// `:DB/:CHUNK/<dim>/<x>/<z>[/<tag>]`
    DbChunk { dim: u8, x: i32, z: i32, tag: Option<u8> },
    /// `:DB/<hex>` — raw LevelDB key.
    DbRaw(Vec<u8>),
    /// Plain world file path (relative, `/`-separated).
    File(String),
}

#[derive(Debug, thiserror::Error)]
#[error("invalid virtual path: {0}")]
pub struct VPathParseError(String);

impl FromStr for VPath {
    type Err = VPathParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(rest) = s.strip_prefix(":DB/") {
            if let Some(chunk) = rest.strip_prefix(":CHUNK/") {
                let mut parts = chunk.split('/');
                let dim: u8 = parts
                    .next()
                    .and_then(|p| p.parse().ok())
                    .ok_or_else(|| VPathParseError(format!("bad dim in {s:?}")))?;
                let x: i32 = parts
                    .next()
                    .and_then(|p| p.parse().ok())
                    .ok_or_else(|| VPathParseError(format!("bad x in {s:?}")))?;
                let z: i32 = parts
                    .next()
                    .and_then(|p| p.parse().ok())
                    .ok_or_else(|| VPathParseError(format!("bad z in {s:?}")))?;
                let tag = parts
                    .next()
                    .map(|p| u8::from_str_radix(p, 16))
                    .transpose()
                    .map_err(|_| VPathParseError(format!("bad tag in {s:?}")))?;
                return Ok(Self::DbChunk { dim, x, z, tag });
            }
            let raw = hex_decode(rest)
                .ok_or_else(|| VPathParseError(format!("bad hex key in {s:?}")))?;
            return Ok(Self::DbRaw(raw));
        }
        if s.starts_with('/') || !s.contains(":DB") {
            return Ok(Self::File(s.trim_start_matches('/').to_string()));
        }
        Err(VPathParseError(format!("unrecognized virtual path {s:?}")))
    }
}

impl fmt::Display for VPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DbChunk { dim, x, z, tag } => match tag {
                Some(t) => write!(f, ":DB/:CHUNK/{dim}/{x}/{z}/{t:02x}"),
                None => write!(f, ":DB/:CHUNK/{dim}/{x}/{z}"),
            },
            Self::DbRaw(key) => {
                write!(f, ":DB/")?;
                for b in key {
                    write!(f, "{b:02x}")?;
                }
                Ok(())
            }
            Self::File(p) => write!(f, "/{p}"),
        }
    }
}

impl VPath {
    /// Map to the store node the path addresses, if it is node-shaped.
    pub fn to_node(&self) -> Option<WorldNode> {
        match self {
            Self::DbChunk { dim, x, z, .. } => Some(WorldNode::Chunk { dim: *dim, x: *x, z: *z }),
            Self::DbRaw(key) => Some(WorldNode::DbKey(key.clone())),
            Self::File(p) => match p.as_str() {
                "level.dat" => Some(WorldNode::LevelDat),
                _ => None,
            },
        }
    }
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_paths() {
        let p: VPath = ":DB/:CHUNK/0/10/-3".parse().unwrap();
        assert_eq!(p, VPath::DbChunk { dim: 0, x: 10, z: -3, tag: None });
        assert_eq!(p.to_string(), ":DB/:CHUNK/0/10/-3");

        let p: VPath = ":DB/:CHUNK/1/5/6/2f".parse().unwrap();
        assert_eq!(p.tag_of_chunk(), Some(0x2f));
    }

    #[test]
    fn raw_and_file_paths() {
        let p: VPath = ":DB/0a0b".parse().unwrap();
        assert_eq!(p, VPath::DbRaw(vec![0x0a, 0x0b]));
        assert_eq!(p.to_string(), ":DB/0a0b");

        let p: VPath = "/region/r.0.0.mca".parse().unwrap();
        assert_eq!(p, VPath::File("region/r.0.0.mca".into()));
    }
}

impl VPath {
    /// The LevelDB record tag byte when this is a `:CHUNK` path with a tag.
    pub fn tag_of_chunk(&self) -> Option<u8> {
        match self {
            Self::DbChunk { tag, .. } => *tag,
            _ => None,
        }
    }
}
