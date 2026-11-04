//! Detect what a path points at and produce a world summary.

use std::path::{Path, PathBuf};

use omt_nbt::{BedrockLevelDat, ByteOrder, Container, Nbt};
use thiserror:: Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldSource {
    JavaFolder,
    BedrockFolder,
    /// `.mcworld` / `.mctemplate` / zip of a Bedrock (or Java) world.
    Archive,
}

#[derive(Debug, Error)]
pub enum DetectError {
    #[error("{0}")]
    NotAWorld(String),
    #[error("unreadable {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("bad level.dat: {0}")]
    Nbt(#[from] omt_nbt::Error),
}

/// Which side of the fork a path is, plus the parsed level.dat when found.
#[derive(Debug)]
pub enum Detected {
    BedrockFolder {
        root: PathBuf,
        level_dat: BedrockLevelDat,
    },
    JavaFolder {
        root: PathBuf,
        level_dat: Nbt,
    },
    Archive(PathBuf),
}

/// `level.dat` flavor check: Bedrock begins with an 8-byte header whose first
/// u32 LE is a small storage version (8..=10 today); Java level.dat is
/// gzip-compressed big-endian NBT.
pub fn detect(path: &Path) -> Result<Detected, DetectError> {
    if path.is_file() {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".mcworld") || lower.ends_with(".mctemplate") || lower.ends_with(".zip") {
            return Ok(Detected::Archive(path.to_path_buf()));
        }
        return Err(DetectError::NotAWorld(format!(
            "{} is a file but not a .mcworld/.mctemplate/.zip world archive",
            path.display()
        )));
    }
    if !path.is_dir() {
        return Err(DetectError::NotAWorld(format!("{} does not exist", path.display())));
    }

    let level_dat = path.join("level.dat");
    if level_dat.is_file() {
        let bytes = std::fs::read(&level_dat)
            .map_err(|source| DetectError::Io { path: level_dat.clone(), source })?;
        if looks_bedrock(&bytes) {
            let ld = BedrockLevelDat::parse(&bytes)?;
            return Ok(Detected::BedrockFolder { root: path.to_path_buf(), level_dat: ld });
        }
        // Java level.dat: gzip (or raw) big-endian.
        let (payload, _) = Container::decode(&bytes)?;
        let nbt = omt_nbt::read(&payload, ByteOrder::Big)?;
        return Ok(Detected::JavaFolder { root: path.to_path_buf(), level_dat: nbt });
    }
    Err(DetectError::NotAWorld(format!(
        "{} has no level.dat; not a Java or Bedrock world folder",
        path.display()
    )))
}

/// Bedrock marker: storage version 1..=64 in the first u32 LE *and* a payload
/// length that roughly fits the file. (Java gzip starts 0x1f 0x8b so the two
/// can never collide.)
pub fn looks_bedrock(bytes: &[u8]) -> bool {
    if bytes.len() < 8 {
        return false;
    }
    let ver = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let len = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    (1..=64).contains(&ver) && 8 + len <= bytes.len() + 16
}

/// Edition-neutral summary for the world browser / `omt inspect`.
#[derive(Debug, Clone)]
pub struct WorldSummary {
    pub edition: crate::Edition,
    pub source: WorldSource,
    pub root: PathBuf,
    pub name: String,
    /// Java `DataVersion` (folder level.dat) when known.
    pub java_data_version: Option<u32>,
    /// Bedrock storage version header when known.
    pub bedrock_storage_version: Option<u32>,
    /// Bedrock `lastOpenedWithVersion` when known.
    pub bedrock_last_opened: Option<[i32; 5]>,
    pub gamemode: Option<i32>,
    pub time_played_seconds: Option<i64>,
    pub spawn: Option<(i32, i32)>,
    pub icon: Option<PathBuf>,
    /// True when the world is below the registry read floor (warn; writes refused).
    pub below_floor: bool,
}

impl WorldSummary {
    pub fn from_detected(detected: &Detected) -> Result<Self, DetectError> {
        let registry = omt_versions::registry();
        match detected {
            Detected::BedrockFolder { root, level_dat } => {
                let below_floor = registry.check_bedrock_storage(level_dat.storage_version).is_err();
                Ok(Self {
                    edition: crate::Edition::Bedrock,
                    source: WorldSource::BedrockFolder,
                    root: root.clone(),
                    name: level_dat
                        .level_name()
                        .map(str::to_owned)
                        .or_else(|| read_levelname_txt(root))
                        .unwrap_or_else(|| "(unnamed)".into()),
                    java_data_version: None,
                    bedrock_storage_version: Some(level_dat.storage_version),
                    bedrock_last_opened: level_dat.last_opened_with_version(),
                    gamemode: level_dat.game_type(),
                    time_played_seconds: level_dat.time_played(),
                    spawn: level_dat.spawn_xz(),
                    icon: root
                        .join("world_icon.jpeg")
                        .is_file()
                        .then(|| root.join("world_icon.jpeg")),
                    below_floor,
                })
            }
            Detected::JavaFolder { root, level_dat } => {
                // Java nests fields under a root `Data` compound.
                let data = level_dat.tag.get("Data").unwrap_or(&level_dat.tag);
                let dv = data.get("DataVersion").and_then(|t| t.as_i64()).map(|v| v as u32);
                let below_floor = dv.map(|v| registry.check_java(v).is_err()).unwrap_or(false);
                Ok(Self {
                    edition: crate::Edition::Java,
                    source: WorldSource::JavaFolder,
                    root: root.clone(),
                    name: data
                        .get("LevelName")
                        .and_then(|t| t.as_str())
                        .unwrap_or("(unnamed)")
                        .to_string(),
                    java_data_version: dv,
                    bedrock_storage_version: None,
                    bedrock_last_opened: None,
                    gamemode: level_dat
                        .tag
                        .get("GameType")
                        .and_then(|t| t.as_i64())
                        .map(|v| v as i32),
                    time_played_seconds: level_dat.tag.get("Time").and_then(|t| t.as_i64()),
                    spawn: match (
                        level_dat.tag.get("SpawnX").and_then(|t| t.as_i64()),
                        level_dat.tag.get("SpawnZ").and_then(|t| t.as_i64()),
                    ) {
                        (Some(x), Some(z)) => Some((x as i32, z as i32)),
                        _ => None,
                    },
                    icon: None,
                    below_floor,
                })
            }
            Detected::Archive(p) => Ok(Self {
                edition: crate::Edition::Bedrock, // refined when the zip is opened (M1)
                source: WorldSource::Archive,
                root: p.clone(),
                name: p
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("archive")
                    .to_string(),
                java_data_version: None,
                bedrock_storage_version: None,
                bedrock_last_opened: None,
                gamemode: None,
                time_played_seconds: None,
                spawn: None,
                icon: None,
                below_floor: false,
            }),
        }
    }
}

fn read_levelname_txt(root: &Path) -> Option<String> {
    let raw = std::fs::read(root.join("levelname.txt")).ok()?;
    // levelname.txt is UTF-8 on modern Bedrock; strip BOM and CRLF.
    let s = String::from_utf8_lossy(&raw);
    let t = s.trim_start_matches('\u{feff}').trim_end();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}
