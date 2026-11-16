//! Planning and executing a prune on a (workspace copy of a) world.
//!
//! Java: every region family of the dimension (`region/`, `entities/`,
//! `poi/`) is rebuilt with [`omt_java::RegionFile::retain`] — surviving
//! chunks are copied sector-verbatim, dead sectors reclaimed, emptied
//! regions deleted (UMT: "pruning deletes entities and points of interest
//! in the chunks"). Bedrock: one pass over the LevelDB deletes the columns'
//! records, `digp` lists and the actors they reference; **Optimize** then
//! compacts the database so the space is actually returned.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use omt_bedrock::BedrockWorld;
use omt_java::{JavaWorld, RegionFile};
use omt_nbt::ByteOrder;
use omt_world::{Edition, WorldError};
use rayon::prelude::*;
use thiserror::Error;

use crate::layer::Selection;

#[derive(Debug, Error)]
pub enum PruneError {
    #[error(transparent)]
    World(#[from] WorldError),
    #[error("I/O on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("not a Minecraft world: {0}")]
    NotAWorld(String),
    #[error("{0}")]
    Unsupported(&'static str),
}

pub type PruneResult<T> = Result<T, PruneError>;

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> PruneError + '_ {
    move |source| PruneError::Io { path: path.to_path_buf(), source }
}

/// One chunk column as the pruner sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkInfo {
    pub x: i32,
    pub z: i32,
    /// `InhabitedTime` in ticks (Java); `None` on Bedrock or when the scan
    /// was asked not to read chunk payloads.
    pub inhabited: Option<i64>,
}

/// The chunks a selection would delete — shown to the user before Prune.
#[derive(Debug, Clone, Default)]
pub struct PrunePlan {
    pub dim: u8,
    pub total: usize,
    pub selected: Vec<(i32, i32)>,
}

impl PrunePlan {
    pub fn kept(&self) -> usize {
        self.total - self.selected.len()
    }
}

/// What a prune actually removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    pub chunks_deleted: usize,
    /// Java: region/entities/poi files rewritten / deleted.
    pub files_rewritten: usize,
    pub files_deleted: usize,
    /// Bedrock: records and actors removed.
    pub records_deleted: usize,
    pub actors_deleted: usize,
}

/// A world opened for pruning. Always operate on a workspace copy.
pub enum PruneWorld {
    Java(Box<JavaWorld>),
    Bedrock(Box<BedrockWorld>),
}

impl PruneWorld {
    /// Open writable. Detects the edition from the folder contents.
    pub fn open(root: &Path) -> PruneResult<Self> {
        match omt_world::detect(root).map_err(|e| PruneError::NotAWorld(e.to_string()))? {
            omt_world::Detected::JavaFolder { .. } => Ok(Self::Java(Box::new(JavaWorld::open(root)?))),
            omt_world::Detected::BedrockFolder { .. } => {
                Ok(Self::Bedrock(Box::new(BedrockWorld::open_writable(root)?)))
            }
            omt_world::Detected::Archive(p) => Err(PruneError::NotAWorld(format!(
                "{} is an archive; extract it into a workspace first",
                p.display()
            ))),
        }
    }

    pub fn edition(&self) -> Edition {
        match self {
            Self::Java(_) => Edition::Java,
            Self::Bedrock(_) => Edition::Bedrock,
        }
    }

    /// Present chunk columns; `with_time` reads Java chunk payloads for
    /// `InhabitedTime` (parallel per region).
    pub fn scan(&self, dim: u8, with_time: bool) -> PruneResult<Vec<ChunkInfo>> {
        match self {
            Self::Bedrock(w) => Ok(w
                .list_chunks(dim)?
                .into_iter()
                .map(|(x, z)| ChunkInfo { x, z, inhabited: None })
                .collect()),
            Self::Java(w) => {
                let Some(dir) = w.dimension_dir(dim) else { return Ok(Vec::new()) };
                let regions = region_files(&dir)?;
                let per: Vec<PruneResult<Vec<ChunkInfo>>> = regions
                    .par_iter()
                    .map(|(path, rx, rz)| scan_region(path, *rx, *rz, with_time))
                    .collect();
                let mut out = Vec::new();
                for r in per {
                    out.extend(r?);
                }
                out.sort_unstable_by_key(|c| (c.x, c.z));
                Ok(out)
            }
        }
    }

    /// Resolve the selection against the world's present chunks.
    pub fn plan(&self, dim: u8, selection: &Selection) -> PruneResult<PrunePlan> {
        let chunks = self.scan(dim, selection.needs_inhabited_time())?;
        let selected = chunks
            .iter()
            .filter(|c| selection.selects(c.x, c.z, c.inhabited))
            .map(|c| (c.x, c.z))
            .collect();
        Ok(PrunePlan { dim, total: chunks.len(), selected })
    }

    /// Delete the planned chunks.
    pub fn execute(&self, plan: &PrunePlan) -> PruneResult<PruneReport> {
        let doomed: HashSet<(i32, i32)> = plan.selected.iter().copied().collect();
        if doomed.is_empty() {
            return Ok(PruneReport::default());
        }
        match self {
            Self::Bedrock(w) => {
                let stats = w.delete_columns(plan.dim, &doomed)?;
                Ok(PruneReport {
                    chunks_deleted: stats.columns,
                    records_deleted: stats.records,
                    actors_deleted: stats.actors,
                    ..Default::default()
                })
            }
            Self::Java(w) => {
                let mut report = PruneReport::default();
                for kind in ["region", "entities", "poi"] {
                    let Some(dir) = w.dimension_subdir(plan.dim, kind) else { continue };
                    let part = prune_region_dir(&dir, &doomed)?;
                    if kind == "region" {
                        report.chunks_deleted = part.chunks_deleted;
                    }
                    report.files_rewritten += part.files_rewritten;
                    report.files_deleted += part.files_deleted;
                }
                Ok(report)
            }
        }
    }

    /// UMT's **Optimize** tool: LevelDB compaction (Bedrock only). Java
    /// regions are already compacted by [`Self::execute`].
    pub fn optimize(&self) -> PruneResult<()> {
        match self {
            Self::Bedrock(w) => Ok(w.compact()?),
            Self::Java(_) => Err(PruneError::Unsupported(
                "Optimize is a Bedrock LevelDB compaction; Java regions are compacted when pruned",
            )),
        }
    }
}

fn region_files(dir: &Path) -> PruneResult<Vec<(PathBuf, i32, i32)>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io(dir)(e)),
    };
    let mut out: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let (rx, rz) = RegionFile::coords_from_name(&p)?;
            Some((p, rx, rz))
        })
        .collect();
    out.sort();
    Ok(out)
}

fn scan_region(path: &Path, rx: i32, rz: i32, with_time: bool) -> PruneResult<Vec<ChunkInfo>> {
    let bytes = std::fs::read(path).map_err(io(path))?;
    let Ok(region) = RegionFile::from_bytes(path, &bytes) else {
        return Ok(Vec::new()); // lenient: unreadable region lists nothing
    };
    Ok(region
        .chunk_coords(rx, rz)
        .into_iter()
        .map(|(x, z)| {
            let inhabited = if with_time {
                // A chunk that cannot be decoded reports 0 ticks — it was
                // certainly never meaningfully inhabited.
                Some(
                    region
                        .read_raw(&bytes, x, z)
                        .ok()
                        .and_then(|raw| omt_nbt::read(&raw, ByteOrder::Big).ok())
                        .and_then(|n| n.tag.get("InhabitedTime").and_then(|t| t.as_i64()))
                        .unwrap_or(0),
                )
            } else {
                None
            };
            ChunkInfo { x, z, inhabited }
        })
        .collect())
}

fn prune_region_dir(dir: &Path, doomed: &HashSet<(i32, i32)>) -> PruneResult<PruneReport> {
    let mut report = PruneReport::default();
    for (path, rx, rz) in region_files(dir)? {
        // Skip regions the selection cannot touch without reading them.
        let touches = (0..32).any(|lx| (0..32).any(|lz| doomed.contains(&(rx * 32 + lx, rz * 32 + lz))));
        if !touches {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(io(&path))?;
        let Ok(region) = RegionFile::from_bytes(&path, &bytes) else { continue };
        let kept = region.retain(&bytes, rx, rz, |x, z| !doomed.contains(&(x, z)));
        if kept.removed.is_empty() {
            continue;
        }
        report.chunks_deleted += kept.removed.len();
        for (x, z) in &kept.removed_external {
            let mcc = dir.join(format!("c.{x}.{z}.mcc"));
            if mcc.is_file() {
                std::fs::remove_file(&mcc).map_err(io(&mcc))?;
            }
        }
        match kept.image {
            Some(image) => {
                let tmp = path.with_extension("mca.omt_tmp");
                std::fs::write(&tmp, &image).map_err(io(&tmp))?;
                std::fs::rename(&tmp, &path).map_err(io(&path))?;
                report.files_rewritten += 1;
            }
            None => {
                std::fs::remove_file(&path).map_err(io(&path))?;
                report.files_deleted += 1;
            }
        }
    }
    Ok(report)
}
