//! `JavaWorld`: folder-backed Java Edition store (Anvil + dims + playerdata).

use std::path::{Path, PathBuf};

use omt_nbt::{ByteOrder, Container, Nbt, Tag};
use omt_world::{Chunk, Dimension, WorldError, WorldNode, WorldResult, WorldStore};

use crate::region::RegionFile;

/// Directory layout per dimension inside a Java world folder.
pub fn dimension_dir(root: &Path, dim: u8) -> PathBuf {
    match Dimension::from_id(dim) {
        Some(Dimension::Overworld) | None => root.join("region"),
        Some(Dimension::Nether) => root.join("DIM-1").join("region"),
        Some(Dimension::End) => root.join("DIM1").join("region"),
    }
}

pub struct JavaWorld {
    pub root: PathBuf,
    pub level_dat: Nbt,
}

impl JavaWorld {
    pub fn open(root: &Path) -> WorldResult<Self> {
        let bytes = std::fs::read(root.join("level.dat")).map_err(WorldError::Io)?;
        let (payload, _) = Container::decode(&bytes)
            .map_err(|e| WorldError::Corrupt { context: "level.dat container".into(), source: Box::new(e) })?;
        let level_dat = omt_nbt::read(&payload, ByteOrder::Big)
            .map_err(|e| WorldError::Corrupt { context: "level.dat".into(), source: Box::new(e) })?;
        Ok(Self { root: root.to_path_buf(), level_dat })
    }

    /// Fields live under the root `Data` compound (with a legacy flat-root
    /// fallback for very old worlds).
    pub fn data(&self) -> &Tag {
        self.level_dat.tag.get("Data").unwrap_or(&self.level_dat.tag)
    }

    pub fn data_version(&self) -> Option<u32> {
        self.data().get("DataVersion").and_then(|t| t.as_i64()).map(|v| v as u32)
    }

    fn regions(&self, dim: u8) -> WorldResult<Vec<PathBuf>> {
        let dir = dimension_dir(&self.root, dim);
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
        let path = dimension_dir(&self.root, dim).join(format!("r.{rx}.{rz}.mca"));
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

    /// Player `.dat` files (uuid-named).
    pub fn list_players(&self) -> WorldResult<Vec<String>> {
        let dir = self.root.join("playerdata");
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return Ok(out),
        };
        for entry in entries.flatten() {
            if let Some(name) = entry.path().file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".dat") {
                    out.push(name.trim_end_matches(".dat").to_string());
                }
            }
        }
        out.sort();
        Ok(out)
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
                        let name = entry
                            .get("Name")
                            .and_then(|t| t.as_str())
                            .unwrap_or("minecraft:air")
                            .to_string();
                        let mut properties = Vec::new();
                        if let Some(Tag::Compound(props)) = entry.get("Properties") {
                            for (k, v) in props {
                                if let Tag::String(s) = v {
                                    properties.push((k.clone(), s.clone()));
                                }
                            }
                        }
                        palette.push(omt_world::BlockState { name, properties });
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

/// Unpack Java's MSB-first packed `data` long array (4096 values).
pub fn unpack_java_indices(packed: &[i64], palette_len: usize) -> Vec<u16> {
    let bits = java_index_bits(palette_len);
    if bits == 0 || packed.is_empty() {
        return vec![0u16; 4096];
    }
    let mut out = Vec::with_capacity(4096);
    let mut value_bits = 0u32;
    let mut acc: u64 = 0;
    for &l in packed {
        let word = l as u64;
        acc |= word << value_bits;
        value_bits += 64;
        while value_bits >= bits && out.len() < 4096 {
            out.push((acc & ((1u64 << bits) - 1)) as u16);
            acc >>= bits;
            value_bits -= bits;
        }
    }
    out.resize(4096, 0);
    out
}

fn java_index_bits(palette_len: usize) -> u32 {
    if palette_len <= 1 {
        return 0;
    }
    let mut bits = 4u32; // vanilla minimum
    while (1usize << bits) < palette_len && bits < 16 {
        bits += 1;
    }
    bits
}

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
                std::fs::read(self.root.join("playerdata").join(format!("{name}.dat")))
                    .map_err(WorldError::Io)
            }
            WorldNode::Chunk { x, z, .. } => match self.read_chunk_nbt(0, *x, *z)? {
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
