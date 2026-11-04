//! `BedrockWorld`: the LevelDB-backed world store.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bedrock_leveldb::{Db, OpenOptions, WriteBatch, WriteOptions};
use omt_nbt::{BedrockLevelDat, ByteOrder, Nbt};
use omt_world::{Chunk, Dimension, Slice, WorldNode, WorldStore, WorldError, WorldResult};
use omt_world::BlockState;

use crate::keys::{self, tag, DbKey};
use crate::subchunk::{Data3d, Subchunk};

/// An open Bedrock world. The LevelDB handle is wrapped in a `Mutex` because
/// the store must be `Send + Sync` while leveldb writes are serialized.
pub struct BedrockWorld {
    pub root: PathBuf,
    pub level_dat: BedrockLevelDat,
    pub levelname: Option<String>,
    db: Mutex<Db>,
}

impl BedrockWorld {
    /// Open read-only — browsing, inspecting, converting. Never mutates.
    pub fn open_read_only(root: &Path) -> WorldResult<Self> {
        let level_bytes = std::fs::read(root.join("level.dat"))
            .map_err(WorldError::Io)
            .map_err(|e| WorldError::Corrupt { context: "level.dat".into(), source: Box::new(e) })?;
        let level_dat = BedrockLevelDat::parse(&level_bytes)
            .map_err(|e| WorldError::Corrupt { context: "level.dat".into(), source: Box::new(e) })?;
        let db = Db::open(
            root.join("db"),
            OpenOptions { read_only: true, ..Default::default() },
        )
        .map_err(|e| WorldError::Store(format!("leveldb open: {e}")))?;
        let levelname = std::fs::read_to_string(root.join("levelname.txt"))
            .ok()
            .map(|s| s.trim_start_matches('\u{feff}').trim_end().to_string());
        Ok(Self { root: root.to_path_buf(), level_dat, levelname, db: Mutex::new(db) })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Db> {
        self.db.lock().expect("leveldb mutex poisoned")
    }

    /// Open writable — converter write paths (M3). Mutates the world: pair
    /// with the copy-on-open workflow (copy the world folder first).
    pub fn open_writable(root: &Path) -> WorldResult<Self> {
        let level_bytes = std::fs::read(root.join("level.dat"))
            .map_err(WorldError::Io)
            .map_err(|e| WorldError::Corrupt { context: "level.dat".into(), source: Box::new(e) })?;
        let level_dat = BedrockLevelDat::parse(&level_bytes)
            .map_err(|e| WorldError::Corrupt { context: "level.dat".into(), source: Box::new(e) })?;
        let db = Db::open(
            root.join("db"),
            OpenOptions { read_only: false, ..Default::default() },
        )
        .map_err(|e| WorldError::Store(format!("leveldb open: {e}")))?;
        let levelname = std::fs::read_to_string(root.join("levelname.txt"))
            .ok()
            .map(|s| s.trim_start_matches('\u{feff}').trim_end().to_string());
        Ok(Self { root: root.to_path_buf(), level_dat, levelname, db: Mutex::new(db) })
    }

    /// Raw point-read of any database key (LevelDB browser, tests).
    pub fn raw_get(&self, key: &[u8]) -> WorldResult<Option<Vec<u8>>> {
        self.get(key)
    }

    /// Raw batched put of arbitrary key/value pairs (converter write path).
    /// The caller owns key layout (`omt_bedrock::keys` builders).
    pub fn raw_put_many(&self, kvs: &[(Vec<u8>, Vec<u8>)]) -> WorldResult<()> {
        if kvs.is_empty() {
            return Ok(());
        }
        let mut batch = WriteBatch::new();
        for (k, v) in kvs {
            batch.put(k.clone(), v.clone());
        }
        self.db()
            .write(batch, WriteOptions::default())
            .map_err(|e| WorldError::Store(format!("leveldb write: {e}")))
    }

    fn get(&self, key: &[u8]) -> WorldResult<Option<Vec<u8>>> {
        self.db()
            .get(key)
            .map(|v| v.map(|b| b.to_vec()))
            .map_err(|e| WorldError::Store(format!("leveldb get: {e}")))
    }

    /// All chunk columns present in a dimension (columns with a subchunk or
    /// version record). Deterministic order.
    pub fn list_chunks(&self, dim: u8) -> WorldResult<Vec<(i32, i32)>> {
        let snapshot = self
            .db()
            .snapshot()
            .map_err(|e| WorldError::Store(format!("leveldb snapshot: {e}")))?;
        let mut found = std::collections::BTreeSet::new();
        for (k, _) in snapshot.iter() {
            match keys::decode_key(&k) {
                DbKey::Subchunk { x, z, dim: d, .. } if d == dim => {
                    found.insert((x, z));
                }
                DbKey::ChunkRecord { x, z, dim: d, tag: t, .. } if d == dim
                    && (t == tag::VERSION || t == tag::DATA_3D || t == tag::FINALIZED_STATE) => {
                        found.insert((x, z));
                    }
                _ => {}
            }
        }
        Ok(found.into_iter().collect())
    }

    /// Chunk version byte (0x2C record) when present.
    pub fn chunk_version(&self, dim: u8, x: i32, z: i32) -> WorldResult<Option<u8>> {
        Ok(self.get(&keys::version_key(x, z, dim))?.map(|v| v.first().copied().unwrap_or(0)))
    }

    /// Raw key/value pairs of a chunk column (all record tags), for browsing.
    pub fn chunk_records(
        &self,
        dim: u8,
        x: i32,
        z: i32,
    ) -> WorldResult<Vec<(Vec<u8>, Vec<u8>)>> {
        let base = keys::chunk_base(x, z, dim);
        let snapshot = self
            .db()
            .snapshot()
            .map_err(|e| WorldError::Store(format!("leveldb snapshot: {e}")))?;
        Ok(snapshot
            .scan_prefix(&base)
            .filter(|(k, _)| {
                let rest = &k[base.len()..];
                // Record tag, possibly + subchunk y; nothing longer.
                matches!(rest.len(), 1 | 2)
            })
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect())
    }

    /// Full decoded chunk.
    pub fn read_chunk_full(&self, dim: u8, x: i32, z: i32) -> WorldResult<Option<Chunk>> {
        let version = self.chunk_version(dim, x, z)?.unwrap_or(0);
        let has_data3d = self.get(&keys::data3d_key(x, z, dim))?.is_some();
        let has_finalized =
            self.get(&keys::chunk_record(x, z, dim, tag::FINALIZED_STATE))?.is_some();
        let mut chunk = Chunk {
            x,
            z,
            dim,
            data_version: version as u32,
            status: Some("full".into()),
            ..Default::default()
        };

        // data3D: heightmap + biomes.
        if let Some(raw) = self.get(&keys::data3d_key(x, z, dim))? {
            match Data3d::parse(&raw) {
                Ok(d) => {
                    chunk.heightmaps = vec![(
                        "bedrock_2d".into(),
                        d.heightmap.iter().map(|&h| h as u64).collect(),
                    )];
                    chunk.biomes_3d = true;
                    chunk.bedrock_biomes = Some(d.subchunks);
                }
                Err(e) => {
                    return Err(WorldError::Corrupt {
                        context: format!("data3D at ({x},{z}) dim {dim}"),
                        source: Box::new(e),
                    });
                }
            }
        }

        // Subchunks: probe the plausible Y range. The column exists when it
        // has a version record, data3D, or any subchunk at all.
        let mut any_subchunk = false;
        for y in -4i8..=19 {
            let Some(raw) = self.get(&keys::subchunk_key(x, z, dim, y))? else {
                continue;
            };
            any_subchunk = true;
            let sub = Subchunk::parse(&raw, Some(y)).map_err(|e| {
                WorldError::Corrupt {
                    context: format!("subchunk y={y} at ({x},{z}) dim {dim}"),
                    source: Box::new(e),
                }
            })?;
            if let Some(storage) = sub.storages.first() {
                chunk.slices.insert(
                    y,
                    Slice {
                        palette: storage.palette.clone(),
                        // Normalize Bedrock's on-disk XZY order into the
                        // universal model's YZX (see omt_world::packing);
                        // without this, block_at read transposed blocks.
                        indices: storage
                            .indices
                            .as_ref()
                            .map_or_else(
                                || vec![0u16; 4096],
                                |ix| omt_world::packing::permute_xzy_to_yzx(ix),
                            ),
                    },
                );
            }
        }

        if version == 0 && !has_data3d && !any_subchunk && !has_finalized {
            return Ok(None);
        }

        // Block entities (0x31): a sequence of little-endian NBT roots, one
        // per entity (fixture-verified: trailing-data error revealed the
        // concatenation).
        if let Some(raw) = self.get(&keys::chunk_record(x, z, dim, tag::BLOCK_ENTITIES))? {
            let mut pos = 0usize;
            while pos < raw.len() {
                let (nbt, consumed) = omt_nbt::read_prefix(&raw[pos..], ByteOrder::Little)
                    .map_err(|e| WorldError::Corrupt {
                        context: format!("block entity #{pos} at ({x},{z}) dim {dim}"),
                        source: Box::new(e),
                    })?;
                if consumed == 0 {
                    return Err(WorldError::Corrupt {
                        context: format!("block entities at ({x},{z}) dim {dim}"),
                        source: Box::new(omt_nbt::Error::TrailingData(0)),
                    });
                }
                chunk.block_entities.push(nbt.tag);
                pos += consumed;
            }
        }

        // Actors: digp → actor ids → actorprefix NBT.
        if let Some(digp) = self.get(&keys::digp_key(x, z, dim))? {
            for id_bytes in digp.as_chunks::<8>().0 {
                let mut id = [0u8; 8];
                id.copy_from_slice(id_bytes);
                if let Some(raw) = self.get(&keys::actor_key(id))?
                    && let Ok(nbt) = omt_nbt::read(&raw, ByteOrder::Little) {
                        chunk.entities.push(nbt.tag);
                    }
            }
        }

        Ok(Some(chunk))
    }

    /// Every raw database key/value (LevelDB browser backend).
    pub fn iter_raw(
        &self,
    ) -> WorldResult<Vec<(Vec<u8>, Vec<u8>)>> {
        let snapshot = self
            .db()
            .snapshot()
            .map_err(|e| WorldError::Store(format!("leveldb snapshot: {e}")))?;
        Ok(snapshot.iter().map(|(k, v)| (k.to_vec(), v.to_vec())).collect())
    }
}

impl WorldStore for BedrockWorld {
    fn read_chunk(&self, dim: u8, x: i32, z: i32) -> WorldResult<Option<Chunk>> {
        self.read_chunk_full(dim, x, z)
    }

    fn write_chunk(&self, _dim: u8, _x: i32, _z: i32, _chunk: &Chunk) -> WorldResult<()> {
        Err(WorldError::WriteRefused(
            "Bedrock chunk writing arrives with the copy-on-open workspace (M3)".into(),
        ))
    }

    fn delete_chunk(&self, dim: u8, x: i32, z: i32) -> WorldResult<()> {
        // Enumerate every key with this column's base and delete it,
        // including digp; actor NBT itself is left for actor GC (M5).
        let base = keys::chunk_base(x, z, dim);
        let snapshot = self
            .db()
            .snapshot()
            .map_err(|e| WorldError::Store(format!("leveldb snapshot: {e}")))?;
        let victims: Vec<Vec<u8>> = snapshot
            .scan_prefix(&base)
            .map(|(k, _)| k.to_vec())
            .collect();
        let digp_victim = keys::digp_key(x, z, dim);
        drop(snapshot);
        let db = self.db();
        let mut batch = WriteBatch::new();
        for k in victims {
            batch.delete(k);
        }
        batch.delete(digp_victim);
        db.write(batch, WriteOptions::default())
            .map_err(|e| WorldError::Store(format!("leveldb write: {e}")))?;
        Ok(())
    }

    fn iter_chunks(&self, dim: u8) -> WorldResult<Box<dyn Iterator<Item = (i32, i32)> + '_>> {
        let list = self.list_chunks(dim)?;
        Ok(Box::new(list.into_iter()))
    }

    fn read_raw(&self, node: &WorldNode) -> WorldResult<Vec<u8>> {
        match node {
            WorldNode::LevelDat => std::fs::read(self.root.join("level.dat")).map_err(WorldError::Io),
            WorldNode::DbKey(key) => self
                .get(key)?
                .ok_or_else(|| WorldError::NoSuchNode(node.clone())),
            WorldNode::PlayerFile(name) => {
                let key = if name == "local" {
                    b"~local_player".to_vec()
                } else {
                    name.as_bytes().to_vec()
                };
                self.get(&key)?
                    .ok_or_else(|| WorldError::NoSuchNode(node.clone()))
            }
            WorldNode::Chunk { dim, x, z } => {
                // Raw "chunk" = concatenated subchunk payloads (browser view).
                let mut out = Vec::new();
                for y in -4i8..=19 {
                    if let Some(v) = self.get(&keys::subchunk_key(*x, *z, *dim, y))? {
                        out.extend_from_slice(&v);
                    }
                }
                if out.is_empty() {
                    return Err(WorldError::NoSuchNode(node.clone()));
                }
                Ok(out)
            }
            WorldNode::EntityRegion { .. } => Err(WorldError::NoSuchNode(node.clone())),
        }
    }

    fn flush(&mut self) -> WorldResult<()> {
        self.db()
            .flush()
            .map_err(|e| WorldError::Store(format!("leveldb flush: {e}")))
    }
}

/// Parse an actor/player NBT payload (little-endian, empty root name).
pub fn parse_actor_nbt(raw: &[u8]) -> WorldResult<Nbt> {
    omt_nbt::read(raw, ByteOrder::Little).map_err(|e| {
        WorldError::Corrupt { context: "actor NBT".into(), source: Box::new(e) }
    })
}

/// Dimension for a `digp`/chunk key dim byte with the friendly name.
pub fn dimension_name(dim: u8) -> &'static str {
    Dimension::from_id(dim).map(|d| d.name()).unwrap_or("unknown")
}

/// Convenience: canonical BlockState for air (used when a palette is empty).
pub fn air_state() -> BlockState {
    BlockState::air()
}
