//! NBT documents on disk (M2): load with format provenance, save without
//! losing the container/byte-order/header the file arrived in.
//!
//! Safety model: saves are written to a temp file next to the target, the
//! original is copied to `<name>.omt_backup` (first save only), then the temp
//! file is renamed over the target — an interrupted save can never leave a
//! half-written original.

use std::path::{Path, PathBuf};

use crate::{BedrockLevelDat, ByteOrder, Container, Nbt, Tag};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NbtFormat {
    /// Bedrock `level.dat`: `[u32 LE storage version][u32 LE length]` + LE NBT.
    BedrockLevelDat,
    /// Java-flavored `.dat`: gzip-compressed big-endian NBT.
    JavaGzip,
    /// Uncompressed big-endian.
    RawBigEndian,
    /// Uncompressed little-endian.
    RawLittleEndian,
}

impl NbtFormat {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::BedrockLevelDat => "Bedrock level.dat (8-byte header, little-endian)",
            Self::JavaGzip => "gzip big-endian (Java .dat)",
            Self::RawBigEndian => "raw big-endian",
            Self::RawLittleEndian => "raw little-endian",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("NBT parse error in {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: crate::Error,
    },
    #[error("unknown NBT container in {path}; open it raw via `omt nbt --hex`")]
    UnknownFormat { path: PathBuf },
    #[error("save failed: {0}")]
    Save(String),
}

/// An NBT file loaded for editing: the parsed tree plus everything needed to
/// write it back exactly the way it was found.
#[derive(Debug, Clone)]
pub struct NbtDocument {
    pub path: PathBuf,
    pub format: NbtFormat,
    pub nbt: Nbt,
    /// Bedrock level.dat storage version (format-provenance).
    pub bedrock_storage_version: u32,
}

impl NbtDocument {
    /// Load a file, sniffing its format.
    pub fn load(path: &Path) -> Result<Self, DocumentError> {
        let bytes = std::fs::read(path).map_err(|source| DocumentError::Io { path: path.to_path_buf(), source })?;
        Self::from_bytes(path, &bytes)
    }

    /// Load a raw little-endian payload (caller knows the provenance).
    pub fn load_le(path: &Path) -> Result<Self, DocumentError> {
        let bytes = std::fs::read(path).map_err(|source| DocumentError::Io { path: path.to_path_buf(), source })?;
        let nbt = crate::read(&bytes, ByteOrder::Little)
            .map_err(|source| DocumentError::Parse { path: path.to_path_buf(), source })?;
        Ok(Self { path: path.to_path_buf(), format: NbtFormat::RawLittleEndian, nbt, bedrock_storage_version: 0 })
    }

    pub fn from_bytes(path: &Path, bytes: &[u8]) -> Result<Self, DocumentError> {
        // Bedrock level.dat: small u32 LE version + fitting length.
        if crate::document::looks_like_bedrock_level_dat(bytes) {
            let ld = BedrockLevelDat::parse(bytes)
                .map_err(|source| DocumentError::Parse { path: path.to_path_buf(), source })?;
            return Ok(Self {
                path: path.to_path_buf(),
                format: NbtFormat::BedrockLevelDat,
                nbt: ld.nbt,
                bedrock_storage_version: ld.storage_version,
            });
        }
        match Container::detect(bytes) {
            Container::Gzip => {
                let (payload, _) = Container::decode(bytes)
                    .map_err(|source| DocumentError::Parse { path: path.to_path_buf(), source })?;
                let nbt = crate::read(&payload, ByteOrder::Big)
                    .map_err(|source| DocumentError::Parse { path: path.to_path_buf(), source })?;
                Ok(Self { path: path.to_path_buf(), format: NbtFormat::JavaGzip, nbt, bedrock_storage_version: 0 })
            }
            Container::Raw => {
                // Raw payloads are order-ambiguous at the byte level; default
                // to big-endian (Java convention). Bedrock callers that know
                // their provenance use [`NbtDocument::load_le`].
                match crate::read(bytes, ByteOrder::Big) {
                    Ok(nbt) => Ok(Self {
                        path: path.to_path_buf(),
                        format: NbtFormat::RawBigEndian,
                        nbt,
                        bedrock_storage_version: 0,
                    }),
                    Err(be_err) => match crate::read(bytes, ByteOrder::Little) {
                        Ok(nbt) => Ok(Self {
                            path: path.to_path_buf(),
                            format: NbtFormat::RawLittleEndian,
                            nbt,
                            bedrock_storage_version: 0,
                        }),
                        Err(le_err) => Err(DocumentError::Parse {
                            path: path.to_path_buf(),
                            source: crate::Error::Container(format!(
                                "neither byte order parses (BE: {be_err}; LE: {le_err})"
                            )),
                        }),
                    },
                }
            }
            Container::Zlib => Err(DocumentError::UnknownFormat { path: path.to_path_buf() }),
        }
    }

    /// Serialize in the original format.
    pub fn to_bytes(&self) -> Result<Vec<u8>, DocumentError> {
        match self.format {
            NbtFormat::BedrockLevelDat => {
                let ld = BedrockLevelDat {
                    storage_version: self.bedrock_storage_version,
                    nbt: self.nbt.clone(),
                };
                ld.serialize().map_err(|e| DocumentError::Save(e.to_string()))
            }
            NbtFormat::JavaGzip => {
                let payload = crate::write(&self.nbt, ByteOrder::Big)
                    .map_err(|e| DocumentError::Save(e.to_string()))?;
                Ok(Container::Gzip.encode(&payload))
            }
            NbtFormat::RawBigEndian => {
                crate::write(&self.nbt, ByteOrder::Big).map_err(|e| DocumentError::Save(e.to_string()))
            }
            NbtFormat::RawLittleEndian => {
                crate::write(&self.nbt, ByteOrder::Little).map_err(|e| DocumentError::Save(e.to_string()))
            }
        }
    }

    /// Save safely: backup the original once, write a temp file, rename over.
    /// Returns the backup path when one was created.
    pub fn save(&self) -> Result<Option<PathBuf>, DocumentError> {
        self.save_to(&self.path)
    }

    pub fn save_to(&self, target: &Path) -> Result<Option<PathBuf>, DocumentError> {
        let bytes = self.to_bytes()?;
        let dir = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file_name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document.nbt".to_string());

        // First-save backup of the pristine original.
        let mut backup_created = None;
        let backup_path = dir.join(format!("{file_name}.omt_backup"));
        if target.exists() && !backup_path.exists() {
            std::fs::copy(target, &backup_path)
                .map_err(|source| DocumentError::Io { path: backup_path.clone(), source })?;
            backup_created = Some(backup_path);
        }

        // Atomic-ish write: temp + rename.
        let tmp = dir.join(format!(".{file_name}.omt_tmp"));
        std::fs::write(&tmp, &bytes).map_err(|source| DocumentError::Io { path: tmp.clone(), source })?;
        std::fs::rename(&tmp, target).map_err(|source| DocumentError::Io {
            path: target.to_path_buf(),
            source,
        })?;
        Ok(backup_created)
    }

    pub fn root(&self) -> &Tag {
        &self.nbt.tag
    }

    pub fn root_mut(&mut self) -> &mut Tag {
        &mut self.nbt.tag
    }
}

/// Shared with `omt_world::detect`'s heuristic but inlined here to keep
/// `omt-nbt` dependency-free.
fn looks_like_bedrock_level_dat(bytes: &[u8]) -> bool {
    if bytes.len() < 8 {
        return false;
    }
    let ver = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let len = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    (1..=64).contains(&ver) && 8 + len <= bytes.len() + 16 && bytes.get(8) == Some(&0x0a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omt-doc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bedrock_level_dat_roundtrip_preserves_bytes() {
        let dir = tmp_dir("be");
        let original = BedrockLevelDat {
            storage_version: 10,
            nbt: Nbt { name: "".into(), tag: Tag::Compound(vec![("LevelName".into(), Tag::String("T".into()))]) },
        }
        .serialize()
        .unwrap();
        let path = dir.join("level.dat");
        std::fs::write(&path, &original).unwrap();

        let mut doc = NbtDocument::load(&path).unwrap();
        assert_eq!(doc.format, NbtFormat::BedrockLevelDat);
        // Direct mutation for this test; the edit engine is covered in edit.rs.
        doc.root_mut()
            .insert("BonusChest", Tag::Byte(1)); // direct mutation is fine for this test
        let backup = doc.save().unwrap();
        assert!(backup.is_some(), "first save creates a backup");
        let saved = std::fs::read(&path).unwrap();
        let re = NbtDocument::load(&path).unwrap();
        assert_eq!(re.nbt.tag.get("BonusChest"), Some(&Tag::Byte(1)));
        assert_eq!(re.nbt.tag.get("LevelName").unwrap().as_str(), Some("T"));
        assert_eq!(re.bedrock_storage_version, 10);
        // Backup is the pristine original.
        assert_eq!(std::fs::read(backup.unwrap()).unwrap(), original);
        assert!(saved.len() > original.len());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn java_gzip_roundtrip_preserves_container() {
        let dir = tmp_dir("je");
        let payload = crate::write(
            &Nbt { name: "".into(), tag: Tag::Compound(vec![("Data".into(), Tag::Compound(vec![("v".into(), Tag::Int(7))]))]) },
            ByteOrder::Big,
        )
        .unwrap();
        let path = dir.join("player.dat");
        std::fs::write(&path, Container::Gzip.encode(&payload)).unwrap();

        let mut doc = NbtDocument::load(&path).unwrap();
        assert_eq!(doc.format, NbtFormat::JavaGzip);
        doc.root_mut().insert("Seen", Tag::Long(1));
        doc.save().unwrap();
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(Container::detect(&raw), Container::Gzip, "container preserved");
        let re = NbtDocument::load(&path).unwrap();
        assert_eq!(re.nbt.tag.get("Data").unwrap().get("v"), Some(&Tag::Int(7)));
        assert_eq!(re.nbt.tag.get("Seen"), Some(&Tag::Long(1)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn second_save_does_not_clobber_backup() {
        let dir = tmp_dir("bk");
        let path = dir.join("level.dat");
        let original = BedrockLevelDat {
            storage_version: 10,
            nbt: Nbt { name: "".into(), tag: Tag::Compound(vec![]) },
        }
        .serialize()
        .unwrap();
        std::fs::write(&path, &original).unwrap();
        let mut doc = NbtDocument::load(&path).unwrap();
        doc.root_mut().insert("a", Tag::Byte(1));
        assert!(doc.save().unwrap().is_some());
        doc.root_mut().insert("b", Tag::Byte(2));
        assert!(doc.save().unwrap().is_none(), "second save keeps the first backup");
        let backup = std::fs::read(dir.join("level.dat.omt_backup")).unwrap();
        assert_eq!(backup, original, "backup still the pristine original");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn raw_defaults_be_with_explicit_le_override() {
        // Raw payloads are byte-level ambiguous; detection defaults to BE.
        let be = crate::write(&Nbt { name: "".into(), tag: Tag::Int(1) }, ByteOrder::Big).unwrap();
        let doc = NbtDocument::from_bytes(Path::new("x"), &be).unwrap();
        assert_eq!(doc.format, NbtFormat::RawBigEndian);

        // Known-LE callers (Bedrock contexts) can force the order.
        let dir = tmp_dir("le");
        let path = dir.join("raw.nbt");
        std::fs::write(&path, crate::write(&Nbt { name: "".into(), tag: Tag::Compound(vec![("v".into(), Tag::Int(9))]) }, ByteOrder::Little).unwrap()).unwrap();
        let doc = NbtDocument::load_le(&path).unwrap();
        assert_eq!(doc.format, NbtFormat::RawLittleEndian);
        doc.save().unwrap();
        let re = NbtDocument::load_le(&path).unwrap();
        assert_eq!(re.nbt.tag.get("v"), Some(&Tag::Int(9)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
