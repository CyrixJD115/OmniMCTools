//! `JavaWorld`: folder-backed Java Edition store (Anvil + dims + playerdata).

use std::path::{Path, PathBuf};

use omt_nbt::{ByteOrder, Container, Nbt, Tag};
use omt_world::{Chunk, Dimension, WorldError, WorldNode, WorldResult, WorldStore};

use crate::region::RegionFile;

/// Parse one Java block-palette entry across format eras (verified against
/// the 26.3 fixture, PLAN §0 2026-11-08):
/// - 1.18–1.25: every entry is a `{Name: String, Properties?: {…}}` compound.
/// - 26.3 (DataVersion 5023): sections whose blocks all sit in their default
///   state carry `List<String>` palettes (plain block names, no properties);
///   sections needing properties use compound entries of exactly two shapes —
///   `{ "": name }` (property-less, empty-named string field) and
///   `{ id: name, properties: {…} }`. The capitalized `Name`/`Properties`
///   keys never appear in 26.3 chunks. All shapes are accepted here so both
///   eras read through one path.
///
/// `None` = unrecognized entry shape (malformed) — callers decide whether to
/// default (browse) or produce a typed error (converter).
pub fn java_palette_entry(entry: &Tag) -> Option<omt_world::BlockState> {
    match entry {
        Tag::String(name) => {
            Some(omt_world::BlockState { name: name.clone(), properties: Vec::new() })
        }
        Tag::Compound(fields) => {
            let mut name = None;
            let mut properties = Vec::new();
            for (key, value) in fields {
                match (key.as_str(), value) {
                    ("Name" | "id" | "", Tag::String(s)) => name = Some(s.clone()),
                    ("Properties" | "properties", Tag::Compound(props)) => {
                        for (k, v) in props {
                            if let Tag::String(s) = v {
                                properties.push((k.clone(), s.clone()));
                            }
                        }
                    }
                    _ => {}
                }
            }
            name.map(|name| omt_world::BlockState { name, properties })
        }
        _ => None,
    }
}

/// Directory layout per dimension inside a Java world folder — the classic
/// 1.18–1.25 era shape. Used by writers that create Java target folders
/// (the converter); readers go through [`JavaLayout`]-aware resolution.
pub fn dimension_dir(root: &Path, dim: u8) -> PathBuf {
    match Dimension::from_id(dim) {
        Some(Dimension::Overworld) | None => root.join("region"),
        Some(Dimension::Nether) => root.join("DIM-1").join("region"),
        Some(Dimension::End) => root.join("DIM1").join("region"),
    }
}

/// Entity-region directory (1.18+ split), mirroring [`dimension_dir`]:
/// overworld `entities/`, nether `DIM-1/entities/`, end `DIM1/entities/`.
/// Container and compression are the same Anvil/zlib region format (wiki
/// *Region file format*: scheme 2 is what the official client writes);
/// root shape `{Position:IntArray, DataVersion, Entities}` verified against
/// the 26.3 fixture (PLAN §0, 2026-11-08).
pub fn entity_dimension_dir(root: &Path, dim: u8) -> PathBuf {
    match Dimension::from_id(dim) {
        Some(Dimension::Overworld) | None => root.join("entities"),
        Some(Dimension::Nether) => root.join("DIM-1").join("entities"),
        Some(Dimension::End) => root.join("DIM1").join("entities"),
    }
}

/// Java save-folder layout era. 26.3 (DataVersion 5023) moved every
/// dimension under `dimensions/minecraft/<dim>/{region,entities,poi,data}`
/// and players to `players/{data,stats,advancements}`; 1.18–1.25 worlds use
/// root `region`/`entities`, `DIM-1/`, `DIM1/` and `playerdata/`.
/// Verified byte-level against the 26.3 fixture (PLAN §0, 2026-11-08).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaLayout {
    /// Root `region/`, `DIM-1/`, `DIM1/`, `playerdata/` (1.18–1.25).
    Classic,
    /// `dimensions/minecraft/<dim>/…`, `players/data/` (26.3+).
    Namespaced,
}

impl JavaLayout {
    /// Namespaced when the world root carries a `dimensions/` folder. Datapack
    /// custom dimensions also create that folder on classic-layout worlds, so
    /// detection is a coarse hint only — per-dimension resolution falls back
    /// across layouts, making a mis-detection harmless.
    pub fn detect(root: &Path) -> Self {
        if root.join("dimensions").is_dir() {
            Self::Namespaced
        } else {
            Self::Classic
        }
    }

    /// Directory name of a vanilla dimension inside `dimensions/minecraft/`.
    fn dim_dir_name(dim: u8) -> Option<&'static str> {
        match Dimension::from_id(dim) {
            Some(Dimension::Overworld) => Some("overworld"),
            Some(Dimension::Nether) => Some("the_nether"),
            Some(Dimension::End) => Some("the_end"),
            None => None,
        }
    }
}

/// Resolve a per-dimension folder (`kind`: `region` or `entities`) across
/// both layouts. Candidates are tried in order and the first existing
/// directory wins, so hybrid folders (datapack `dimensions/` on a classic
/// world, or unexplored dimensions) resolve to whatever is actually present;
/// when nothing exists the detected layout's primary path is returned so
/// callers see a consistent (missing) directory.
fn resolve_dim_dir(root: &Path, layout: JavaLayout, dim: u8, kind: &str) -> Option<PathBuf> {
    let name = JavaLayout::dim_dir_name(dim)?;
    let mut candidates = vec![root.join("dimensions").join("minecraft").join(name).join(kind)];
    // Other namespaces carrying the same dimension name (modded replacements
    // of overworld/the_nether/the_end); `minecraft` first, others sorted.
    if let Ok(entries) = std::fs::read_dir(root.join("dimensions")) {
        let mut others: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path().join(name).join(kind))
            .filter(|p| *p != candidates[0])
            .collect();
        others.sort();
        candidates.extend(others);
    }
    candidates.push(entity_or_region_classic(root, dim, kind));
    let primary = match layout {
        JavaLayout::Namespaced => 0,
        JavaLayout::Classic => candidates.len() - 1,
    };
    let index = candidates.iter().position(|p| p.is_dir()).unwrap_or(primary);
    Some(candidates.remove(index))
}

/// Classic-layout directory for one dimension and kind (region vs entities).
fn entity_or_region_classic(root: &Path, dim: u8, kind: &str) -> PathBuf {
    match Dimension::from_id(dim) {
        Some(Dimension::Nether) => root.join("DIM-1").join(kind),
        Some(Dimension::End) => root.join("DIM1").join(kind),
        _ => root.join(kind),
    }
}

pub struct JavaWorld {
    pub root: PathBuf,
    pub level_dat: Nbt,
    /// Save-folder layout era, resolved at open ([`JavaLayout::detect`]).
    pub layout: JavaLayout,
}

impl JavaWorld {
    pub fn open(root: &Path) -> WorldResult<Self> {
        let bytes = std::fs::read(root.join("level.dat")).map_err(WorldError::Io)?;
        let (payload, _) = Container::decode(&bytes)
            .map_err(|e| WorldError::Corrupt { context: "level.dat container".into(), source: Box::new(e) })?;
        let level_dat = omt_nbt::read(&payload, ByteOrder::Big)
            .map_err(|e| WorldError::Corrupt { context: "level.dat".into(), source: Box::new(e) })?;
        Ok(Self { root: root.to_path_buf(), level_dat, layout: JavaLayout::detect(root) })
    }

    /// Fields live under the root `Data` compound (with a legacy flat-root
    /// fallback for very old worlds).
    pub fn data(&self) -> &Tag {
        self.level_dat.tag.get("Data").unwrap_or(&self.level_dat.tag)
    }

    pub fn data_version(&self) -> Option<u32> {
        self.data().get("DataVersion").and_then(|t| t.as_i64()).map(|v| v as u32)
    }

    /// Region directory for a dimension, resolved across both layout eras.
    pub fn dimension_dir(&self, dim: u8) -> Option<PathBuf> {
        resolve_dim_dir(&self.root, self.layout, dim, "region")
    }

    /// Entity-region directory for a dimension, resolved like [`Self::dimension_dir`].
    pub fn entity_dimension_dir(&self, dim: u8) -> Option<PathBuf> {
        resolve_dim_dir(&self.root, self.layout, dim, "entities")
    }

    fn regions(&self, dim: u8) -> WorldResult<Vec<PathBuf>> {
        let dir = self.dimension_dir(dim).unwrap_or_else(|| dimension_dir(&self.root, dim));
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(WorldError::Io(e)),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "mca") {
                out.push(path);
            }
        }
        out.sort();
        Ok(out)
    }

    pub fn list_chunks(&self, dim: u8) -> WorldResult<Vec<(i32, i32)>> {
        let mut out = Vec::new();
        for path in self.regions(dim)? {
            let region = RegionFile::open(&path).map_err(region_err)?;
            if let Some((rx, rz)) = region.region_coords() {
                out.extend(region.chunk_coords(rx, rz));
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    pub fn read_chunk_nbt(&self, dim: u8, x: i32, z: i32) -> WorldResult<Option<Nbt>> {
        // Locate the region containing (x, z).
        let rx = x.div_euclid(32);
        let rz = z.div_euclid(32);
        let path = self
            .dimension_dir(dim)
            .unwrap_or_else(|| dimension_dir(&self.root, dim))
            .join(format!("r.{rx}.{rz}.mca"));
        if !path.is_file() {
            return Ok(None);
        }
        let region = RegionFile::open(&path).map_err(region_err)?;
        if !region.contains(x, z) {
            return Ok(None);
        }
        let bytes = std::fs::read(&path).map_err(WorldError::Io)?;
        region
            .read_chunk(&bytes, x, z)
            .map(Some)
            .map_err(|e| WorldError::Corrupt { context: format!("chunk ({x},{z})"), source: Box::new(e) })
    }

    /// Player `.dat` files (uuid-named): `players/data/` on 26.3+ layouts,
    /// `playerdata/` on classic ones.
    pub fn list_players(&self) -> WorldResult<Vec<String>> {
        let dir = self.players_dir();
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return Ok(out),
        };
        for entry in entries.flatten() {
            if let Some(name) = entry.path().file_name().and_then(|n| n.to_str())
                && name.ends_with(".dat") {
                    out.push(name.trim_end_matches(".dat").to_string());
                }
        }
        out.sort();
        Ok(out)
    }

    /// Player-data directory across layout eras: `players/data` when present
    /// (26.3+), else classic `playerdata`.
    fn players_dir(&self) -> PathBuf {
        let modern = self.root.join("players").join("data");
        if modern.is_dir() {
            modern
        } else {
            self.root.join("playerdata")
        }
    }
}

fn region_err(e: crate::region::RegionError) -> WorldError {
    WorldError::Corrupt { context: "region file".into(), source: Box::new(e) }
}

/// Convert a chunk NBT root into the shared [`Chunk`] model (latest-era:
/// `sections` with `block_states`, 3D `biomes`).
pub fn nbt_to_chunk(dim: u8, x: i32, z: i32, nbt: &Nbt) -> WorldResult<Chunk> {
    let mut chunk = Chunk {
        x,
        z,
        dim,
        data_version: nbt.tag.get("DataVersion").and_then(|t| t.as_i64()).unwrap_or(0) as u32,
        status: nbt
            .tag
            .get("Status")
            .and_then(|t| t.as_str())
            .map(str::to_owned),
        inhabited_time: nbt.tag.get("InhabitedTime").and_then(|t| t.as_i64()).unwrap_or(0),
        ..Default::default()
    };
    if let Some(sections) = nbt.tag.get("sections").and_then(|t| t.as_list()) {
        for section in sections {
            let Some(y) = section.get("Y").and_then(|t| t.as_i64()) else { continue };
            if let Some(bs) = section.get("block_states") {
                let mut palette = Vec::new();
                if let Some(pal) = bs.get("palette").and_then(|t| t.as_list()) {
                    for entry in pal {
                        palette.push(java_palette_entry(entry).unwrap_or_else(omt_world::BlockState::air));
                    }
                }
                let indices = match bs.get("data").and_then(|t| match t {
                    Tag::LongArray(v) => Some(v.clone()),
                    _ => None,
                }) {
                    Some(packed) => unpack_java_indices(&packed, palette.len()),
                    None => vec![0u16; 4096],
                };
                chunk.slices.insert(y as i8, omt_world::Slice { palette, indices });
            }
            if section.get("biomes").is_some() {
                chunk.biomes_3d = true;
            }
        }
    }
    if let Some(Tag::Compound(hm)) = nbt.tag.get("Heightmaps") {
        for (name, value) in hm {
            if let Tag::LongArray(packed) = value {
                chunk
                    .heightmaps
                    .push((name.clone(), packed.iter().map(|&v| v as u64).collect()));
            }
        }
    }
    if let Some(Tag::List(items)) = nbt.tag.get("block_entities") {
        chunk.block_entities = items.clone();
    }
    Ok(chunk)
}

/// Unpack Java's non-spanning packed `data` long array (4096 values).
/// Shared implementation moved to `omt_world::packing` (2026-11-04) so the
/// converter and the store cannot drift; the old local copy used the
/// pre-1.16 continuous-stream layout and was wrong for every supported
/// world (latent until a real Java fixture existed).
pub use omt_world::packing::unpack_java_indices;

impl WorldStore for JavaWorld {
    fn read_chunk(&self, dim: u8, x: i32, z: i32) -> WorldResult<Option<Chunk>> {
        match self.read_chunk_nbt(dim, x, z)? {
            Some(nbt) => Ok(Some(nbt_to_chunk(dim, x, z, &nbt)?)),
            None => Ok(None),
        }
    }

    fn write_chunk(&self, _dim: u8, _x: i32, _z: i32, _chunk: &Chunk) -> WorldResult<()> {
        Err(WorldError::WriteRefused(
            "Java chunk writing arrives with the copy-on-open workspace (M3)".into(),
        ))
    }

    fn delete_chunk(&self, _dim: u8, _x: i32, _z: i32) -> WorldResult<()> {
        Err(WorldError::WriteRefused("Java deletion arrives with the pruner (M5)".into()))
    }

    fn iter_chunks(&self, dim: u8) -> WorldResult<Box<dyn Iterator<Item = (i32, i32)> + '_>> {
        let list = self.list_chunks(dim)?;
        Ok(Box::new(list.into_iter()))
    }

    fn read_raw(&self, node: &WorldNode) -> WorldResult<Vec<u8>> {
        match node {
            WorldNode::LevelDat => std::fs::read(self.root.join("level.dat")).map_err(WorldError::Io),
            WorldNode::PlayerFile(name) => {
                std::fs::read(self.players_dir().join(format!("{name}.dat"))).map_err(WorldError::Io)
            }
            WorldNode::Chunk { dim, x, z } => match self.read_chunk_nbt(*dim, *x, *z)? {
                Some(nbt) => Ok(omt_nbt::write(&nbt, ByteOrder::Big).map_err(|e| {
                    WorldError::Corrupt { context: "re-encode".into(), source: Box::new(e) }
                })?),
                None => Err(WorldError::NoSuchNode(node.clone())),
            },
            WorldNode::EntityRegion { .. } | WorldNode::DbKey(_) => {
                Err(WorldError::NoSuchNode(node.clone()))
            }
        }
    }

    fn flush(&mut self) -> WorldResult<()> {
        Ok(())
    }
}
