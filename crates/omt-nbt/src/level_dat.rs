//! Bedrock `level.dat`: 8-byte header + uncompressed little-endian NBT.
//!
//! Header layout (verified against the supplied fixtures, storage version 10):
//! ```text
//! u32 LE  storage version (current worlds: 10)
//! u32 LE  payload byte length
//! ```

use crate::{ByteOrder, Nbt, Result};
use crate::Error;

#[derive(Debug, Clone)]
pub struct BedrockLevelDat {
    pub storage_version: u32,
    pub nbt: Nbt,
}

impl BedrockLevelDat {
    /// Parse a full `level.dat` file image (header + NBT).
    pub fn parse(buf: &[u8]) -> Result<Self> {
        if buf.len() < 8 {
            return Err(Error::BadLevelDatHeader(format!(
                "file is {} bytes; need at least the 8-byte header",
                buf.len()
            )));
        }
        let storage_version = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let declared = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        let payload = &buf[8..];
        if payload.len() < declared {
            return Err(Error::BadLevelDatHeader(format!(
                "declared payload length {declared} but only {} bytes follow the header",
                payload.len()
            )));
        }
        // Some writers pad the file; parse exactly `declared` bytes.
        let nbt = crate::read(&payload[..declared], ByteOrder::Little)?;
        Ok(Self { storage_version, nbt })
    }

    /// Serialize back to a full file image.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let payload = crate::write(&self.nbt, ByteOrder::Little)?;
        let mut out = Vec::with_capacity(8 + payload.len());
        out.extend_from_slice(&self.storage_version.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&payload);
        Ok(out)
    }

    // ---- convenience accessors over the root compound --------------------

    pub fn level_name(&self) -> Option<&str> {
        self.nbt.tag.get("LevelName")?.as_str()
    }

    /// e.g. [1, 21, 90, 0, 0]
    pub fn last_opened_with_version(&self) -> Option<[i32; 5]> {
        version_array(self.nbt.tag.get("lastOpenedWithVersion"))
    }

    pub fn minimum_compatible_client_version(&self) -> Option<[i32; 5]> {
        version_array(self.nbt.tag.get("MinimumCompatibleClientVersion"))
    }

    /// Seconds the world has been played (`Time` tag).
    pub fn time_played(&self) -> Option<i64> {
        self.nbt.tag.get("Time")?.as_i64()
    }

    pub fn game_type(&self) -> Option<i32> {
        self.nbt.tag.get("GameType")?.as_i64().map(|v| v as i32)
    }

    pub fn spawn_xz(&self) -> Option<(i32, i32)> {
        let x = self.nbt.tag.get("SpawnX")?.as_i64()? as i32;
        let z = self.nbt.tag.get("SpawnZ")?.as_i64()? as i32;
        Some((x, z))
    }
}

fn version_array(tag: Option<&crate::Tag>) -> Option<[i32; 5]> {
    let list = tag?.as_list()?;
    let mut out = [0i32; 5];
    for (i, item) in list.iter().take(5).enumerate() {
        out[i] = item.as_i64()? as i32;
    }
    Some(out)
}
