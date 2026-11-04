//! `omt-versions` — the version registry (PLAN.md §2).
//!
//! One row per Minecraft release replaces UMT's 48-byte format-descriptor
//! ladders. Writer stamps `current`; reader accepts `>= min`; below min →
//! friendly error + refuse to write (read/browse/NBT-edit still allowed).

use std::sync::OnceLock;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EditionVersions {
    /// Current stable Java `DataVersion` the writer stamps.
    pub java_data_version: u32,
    /// Human-readable Java release, e.g. "1.21.8".
    pub java_release: String,
    /// Read floor: 2865 = 1.18 (sections-era). Below → per-world warning.
    pub java_min_data_version: u32,
    /// Bedrock `level.dat` `lastOpenedWithVersion` we stamp on write.
    pub bedrock_level_version: [u32; 5],
    /// Current subchunk palette version (v8/v9 era).
    pub bedrock_subchunk_palette: u8,
    pub bedrock_release: String,
    /// Read floor: subchunk-split era (1.18).
    pub bedrock_min_chunk_version: u8,
}

#[derive(Debug, thiserror::Error)]
#[error("registry error: {0}")]
pub enum RegistryError {
    Ron(String),
}

static REGISTRY: OnceLock<EditionVersions> = OnceLock::new();

/// The registry file embedded at build time. `registry.ron` is the single
/// source of truth; bump it per Minecraft release.
pub const REGISTRY_RON: &str = include_str!("../registry.ron");

pub fn registry() -> &'static EditionVersions {
    REGISTRY.get_or_init(|| load().expect("bundled registry.ron must parse"))
}

pub fn load() -> Result<EditionVersions, RegistryError> {
    ron::from_str(REGISTRY_RON).map_err(|e| RegistryError::Ron(e.to_string()))
}

impl EditionVersions {
    /// Java version gate: `Ok` when readable, `Err(found)` when below floor.
    pub fn check_java(&self, data_version: u32) -> Result<(), u32> {
        if data_version >= self.java_min_data_version {
            Ok(())
        } else {
            Err(data_version)
        }
    }

    /// Bedrock version gate on the `level.dat` storage-version header
    /// (worlds at storage version ≥ 8 are the subchunk-split era).
    pub fn check_bedrock_storage(&self, storage_version: u32) -> Result<(), u32> {
        if storage_version >= 8 {
            Ok(())
        } else {
            Err(storage_version)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_registry_parses() {
        let r = registry();
        assert_eq!(r.java_min_data_version, 2865, "1.18 floor");
        assert!(r.java_data_version >= r.java_min_data_version);
        assert_eq!(r.bedrock_level_version.len(), 5);
    }
}
