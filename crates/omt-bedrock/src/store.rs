//! `BedrockWorld`: the LevelDB-backed world store.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use bedrock_leveldb::{
    CachePolicy, Db, OpenOptions, ReadOptions, ScanMode, VisitorControl, WriteBatch, WriteOptions,
};
use bytes::Bytes;
use rayon::prelude::*;
use omt_nbt::{BedrockLevelDat, ByteOrder, Nbt};
use omt_world::{Chunk, Dimension, Slice, WorldNode, WorldStore, WorldError, WorldResult};
use omt_world::BlockState;

use crate::index::{ColumnIndex, ColumnInfo};
use crate::keys::{self, tag, DbKey};
use crate::subchunk::{Data3d, Subchunk};

/// Per-column results of [`BedrockWorld::read_render_columns`].
pub type ColumnReads = Vec<(i32, i32, WorldResult<Option<Chunk>>)>;
type IndexedValues = Vec<(usize, Option<Bytes>)>;

/// An open Bedrock world. `Db` is `Sync` and serializes writes internally
/// (an `RwLock` around its state), so reads from map tiles, exports and the
/// converter run concurrently.
pub struct BedrockWorld {
    pub root: PathBuf,
    pub level_dat: BedrockLevelDat,
    pub levelname: Option<String>,
    db: Db,
}

/// Marketplace worlds ship an encrypted LevelDB: `db/CURRENT` begins with a
/// binary key-id header instead of the plain `MANIFEST-nnnnnn` line.
fn refuse_encrypted(root: &Path) -> WorldResult<()> {
    match std::fs::read(root.join("db").join("CURRENT")) {
        Ok(bytes) if !bytes.starts_with(b"MANIFEST-") => Err(WorldError::Encrypted),
        _ => Ok(()),
    }
}

impl BedrockWorld {
    /// Open read-only — browsing, inspecting, converting. Never mutates.
    pub fn open_read_only(root: &Path) -> WorldResult<Self> {
        refuse_encrypted(root)?;
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
        Ok(Self { root: root.to_path_buf(), level_dat, levelname, db })
    }

    fn db(&self) -> &Db {
        &self.db
    }

    /// Read options for point/batched reads: the native block + index cache
    /// is used (the crate default bypasses it, re-reading a table's index on
    /// every lookup).
    fn cached_reads() -> ReadOptions {
        ReadOptions { cache_policy: CachePolicy::Use, ..Default::default() }
    }

    /// Key-only scan options: tables decoded in parallel, cache bypassed (a
    /// whole-database pass would only evict useful blocks).
    fn parallel_scan() -> ReadOptions {
        ReadOptions { scan_mode: ScanMode::ParallelTables, ..Default::default() }
    }

    /// Which chunk columns exist (every dimension) and which records they
    /// hold — one streaming, table-parallel **key-only** scan. Memory is
    /// bounded by region count, not database size; this is the bulk
    /// readers' replacement for a whole-database snapshot.
    pub fn column_index(&self) -> WorldResult<ColumnIndex> {
        let (outcome, parts) = self
            .db()
            .for_each_key_partitioned(Self::parallel_scan(), HashMap::new, |part, key| {
                ColumnIndex::visit_key(part, key);
                Ok(VisitorControl::Continue)
            })
            .map_err(|e| WorldError::Store(format!("leveldb key scan: {e}")))?;
        Ok(ColumnIndex::from_partials(parts, outcome.visited as u64))
    }

    /// Exact-key batched reads, preserving input order. Keys are sorted and
    /// split into contiguous slices read in parallel (each slice touches
    /// few tables); tombstones resolve to `None`.
    pub fn get_exact(&self, keys: &[Vec<u8>]) -> WorldResult<Vec<Option<Bytes>>> {
        const SLICE: usize = 768;
        let mut order: Vec<usize> = (0..keys.len()).collect();
        order.sort_unstable_by(|&a, &b| keys[a].cmp(&keys[b]));
        let parts: Vec<WorldResult<IndexedValues>> = order
            .par_chunks(SLICE)
            .map(|idxs| {
                let batch = idxs.iter().map(|&i| Bytes::copy_from_slice(&keys[i]));
                let vals = self
                    .db()
                    .get_many_owned(batch, Self::cached_reads())
                    .map_err(|e| WorldError::Store(format!("leveldb get: {e}")))?;
                Ok(idxs.iter().copied().zip(vals).collect())
            })
            .collect();
        let mut out = vec![None; keys.len()];
        for part in parts {
            for (i, v) in part? {
                out[i] = v;
            }
        }
        Ok(out)
    }

    /// Read and decode the **block data** (version, data3D, subchunks — no
    /// block entities or actors) of many columns, fetching exactly the keys
    /// `info` says exist. Decoding runs in parallel. Callers bound memory
    /// by passing columns in batches (a few hundred at a time).
    pub fn read_render_columns(
        &self,
        dim: u8,
        cols: &[(i32, i32, ColumnInfo)],
    ) -> WorldResult<ColumnReads> {
        let mut keys = Vec::with_capacity(cols.len() * 8);
        let mut owner = Vec::with_capacity(cols.len() * 8);
        for (ci, &(x, z, info)) in cols.iter().enumerate() {
            for k in info.render_keys(x, z, dim) {
                owner.push(ci);
                keys.push(k);
            }
        }
        let vals = self.get_exact(&keys)?;
        let mut recs: Vec<HashMap<Vec<u8>, Vec<u8>>> = vec![HashMap::new(); cols.len()];
        for ((k, v), ci) in keys.into_iter().zip(vals).zip(owner) {
            if let Some(v) = v {
                recs[ci].insert(k, v.to_vec());
            }
        }
        Ok(cols
            .par_iter()
            .zip(recs.into_par_iter())
            .map(|(&(x, z, _), records)| {
                (x, z, Self::decode_column(dim, x, z, &ColumnRecords { records, actors: Vec::new() }))
            })
            .collect())
    }

    /// Open writable — converter write paths (M3). Mutates the world: pair
    /// with the copy-on-open workflow (copy the world folder first).
    pub fn open_writable(root: &Path) -> WorldResult<Self> {
        refuse_encrypted(root)?;
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
        Ok(Self { root: root.to_path_buf(), level_dat, levelname, db })
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
            .get_with(key, Self::cached_reads())
            .map(|v| v.map(|b| b.to_vec()))
            .map_err(|e| WorldError::Store(format!("leveldb get: {e}")))
    }

    /// All chunk columns present in a dimension (columns with a subchunk or
    /// version record). Deterministic order.
    pub fn list_chunks(&self, dim: u8) -> WorldResult<Vec<(i32, i32)>> {
        let index = self.column_index()?;
        let mut found: Vec<(i32, i32)> = index.dim(dim).map(|d| d.chunks().iter().collect()).unwrap_or_default();
        found.sort_unstable();
        Ok(found)
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
        let mut wanted: Vec<Vec<u8>> = self
            .db()
            .collect_prefix_keys_owned(&base, Self::cached_reads())
            .map_err(|e| WorldError::Store(format!("leveldb prefix scan: {e}")))?
            .into_iter()
            // Record tag, possibly + subchunk y; nothing longer (an
            // overworld base is a byte prefix of nether/end keys).
            .filter(|k| matches!(k.len() - base.len(), 1 | 2))
            .map(|k| k.to_vec())
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        let vals = self.get_exact(&wanted)?;
        Ok(wanted.into_iter().zip(vals).filter_map(|(k, v)| Some((k, v?.to_vec()))).collect())
    }

    /// Full decoded chunk.
    pub fn read_chunk_full(&self, dim: u8, x: i32, z: i32) -> WorldResult<Option<Chunk>> {
        let records = self.column_records(dim, x, z)?;
        Self::decode_column(dim, x, z, &records)
    }

    /// The raw records of one chunk column that decoding needs (version,
    /// data3D, finalized, block entities, subchunks −4..19) plus the
    /// column's actors. Call serially (the handle is a mutex); pair with
    /// [`Self::decode_column`] on worker threads.
    pub fn column_records(&self, dim: u8, x: i32, z: i32) -> WorldResult<ColumnRecords> {
        // One batched exact-key read (never a snapshot: `Db::snapshot()`
        // materializes the whole database).
        let mut wanted: Vec<Vec<u8>> = [tag::VERSION, tag::DATA_3D, tag::FINALIZED_STATE, tag::BLOCK_ENTITIES, tag::DATA_2D_LEGACY, tag::DATA_2D_OLD, tag::LEGACY_TERRAIN]
            .iter()
            .map(|&t| keys::chunk_record(x, z, dim, t))
            .collect();
        wanted.extend((-4i8..=19).map(|y| keys::subchunk_key(x, z, dim, y)));
        let record_count = wanted.len();
        wanted.push(keys::digp_key(x, z, dim));
        let vals = self.get_exact(&wanted)?;
        let digp = vals.last().cloned().flatten();
        let mut records = HashMap::new();
        for (k, v) in wanted.into_iter().zip(vals).take(record_count) {
            if let Some(v) = v {
                records.insert(k, v.to_vec());
            }
        }
        let mut actors = Vec::new();
        if let Some(digp) = digp {
            let ids: Vec<Vec<u8>> = digp.as_chunks::<8>().0.iter().map(|id| keys::actor_key(*id)).collect();
            actors.extend(self.get_exact(&ids)?.into_iter().flatten().map(|v| v.to_vec()));
        }
        Ok(ColumnRecords { records, actors })
    }

    /// Decode a column from its raw records (no database access).
    pub fn decode_column(dim: u8, x: i32, z: i32, cols: &ColumnRecords) -> WorldResult<Option<Chunk>> {
        let rec = |k: &[u8]| cols.records.get(k);
        let version = rec(&keys::version_key(x, z, dim)).and_then(|v| v.first().copied()).unwrap_or(0);
        let has_data3d = rec(&keys::data3d_key(x, z, dim)).is_some();
        let has_finalized = rec(&keys::chunk_record(x, z, dim, tag::FINALIZED_STATE)).is_some();
        let mut chunk = Chunk {
            x,
            z,
            dim,
            data_version: version as u32,
            status: Some("full".into()),
            ..Default::default()
        };

        // data3D: heightmap + biomes.
        if let Some(raw) = rec(&keys::data3d_key(x, z, dim)) {
            match Data3d::parse(raw) {
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
            let Some(raw) = rec(&keys::subchunk_key(x, z, dim, y)) else {
                continue;
            };
            any_subchunk = true;
            let sub = Subchunk::parse(raw, Some(y)).map_err(|e| {
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

        // Pre-1.18 biomes (Data2D / Data2DLegacy) when there is no data3D.
        if !has_data3d {
            let d2 = rec(&keys::chunk_record(x, z, dim, tag::DATA_2D_LEGACY)).and_then(|r| crate::legacy::data2d(r, false));
            let d2 = d2.or_else(|| rec(&keys::chunk_record(x, z, dim, tag::DATA_2D_OLD)).and_then(|r| crate::legacy::data2d(r, true)));
            if let Some((heights, biomes)) = d2 {
                chunk.heightmaps = vec![("bedrock_2d".into(), heights.iter().map(|&h| h as u64).collect())];
                chunk.bedrock_biomes = Some(biomes);
            }
        }
        // Pre-1.0 PE: the whole column in one LegacyTerrain record.
        let mut legacy_terrain = false;
        if !any_subchunk
            && let Some((slices, biomes)) = rec(&keys::chunk_record(x, z, dim, tag::LEGACY_TERRAIN)).and_then(|r| crate::legacy::legacy_terrain(r))
        {
            legacy_terrain = true;
            chunk.slices = slices;
            if chunk.bedrock_biomes.is_none() {
                chunk.bedrock_biomes = biomes;
            }
        }

        if version == 0 && !has_data3d && !any_subchunk && !has_finalized && !legacy_terrain {
            return Ok(None);
        }

        // Block entities (0x31): a sequence of little-endian NBT roots, one
        // per entity (fixture-verified: trailing-data error revealed the
        // concatenation).
        if let Some(raw) = rec(&keys::chunk_record(x, z, dim, tag::BLOCK_ENTITIES)) {
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

        // Actors (digp → actorprefix NBT), gathered by `column_records`.
        for raw in &cols.actors {
            if let Ok(nbt) = omt_nbt::read(raw, ByteOrder::Little) {
                chunk.entities.push(nbt.tag);
            }
        }

        Ok(Some(chunk))
    }

    /// Every key with its value length — the LevelDB browser index (values
    /// are fetched on demand, so huge worlds don't load into memory).
    pub fn key_index(&self) -> WorldResult<Vec<(Vec<u8>, usize)>> {
        let mut keys = self.all_keys()?;
        let mut out = Vec::with_capacity(keys.len());
        // Value lengths in bounded slices: values are dropped right away.
        for slice in keys.chunks_mut(65_536) {
            let vals = self.get_exact(slice)?;
            for (k, v) in slice.iter_mut().zip(vals) {
                if let Some(v) = v {
                    out.push((std::mem::take(k), v.len()));
                }
            }
        }
        Ok(out)
    }

    /// Every live key, sorted and deduplicated (key-only parallel scan).
    pub fn all_keys(&self) -> WorldResult<Vec<Vec<u8>>> {
        let (_, parts) = self
            .db()
            .for_each_key_partitioned(Self::parallel_scan(), Vec::new, |v: &mut Vec<Vec<u8>>, key| {
                v.push(key.to_vec());
                Ok(VisitorControl::Continue)
            })
            .map_err(|e| WorldError::Store(format!("leveldb key scan: {e}")))?;
        let mut keys: Vec<Vec<u8>> = parts.into_iter().flatten().collect();
        keys.par_sort_unstable();
        keys.dedup();
        Ok(keys)
    }

    /// Delete raw keys (NBT editor / LevelDB browser).
    pub fn raw_delete_many(&self, keys: &[Vec<u8>]) -> WorldResult<()> {
        let mut batch = WriteBatch::new();
        for k in keys {
            batch.delete(k.clone());
        }
        self.db()
            .write(batch, WriteOptions::default())
            .map_err(|e| WorldError::Store(format!("leveldb write: {e}")))
    }

    /// Every raw database key/value (LevelDB browser backend).
    pub fn iter_raw(
        &self,
    ) -> WorldResult<Vec<(Vec<u8>, Vec<u8>)>> {
        // Inherently the whole database (LevelDB browser / tests), but built
        // straight into the result: no intermediate materialized snapshot.
        let mut keys = self.all_keys()?;
        let mut out = Vec::with_capacity(keys.len());
        for slice in keys.chunks_mut(65_536) {
            let vals = self.get_exact(slice)?;
            for (k, v) in slice.iter_mut().zip(vals) {
                if let Some(v) = v {
                    out.push((std::mem::take(k), v.to_vec()));
                }
            }
        }
        Ok(out)
    }
}

/// Raw records of one chunk column ([`BedrockWorld::column_records`]).
#[derive(Debug, Clone, Default)]
pub struct ColumnRecords {
    pub records: std::collections::HashMap<Vec<u8>, Vec<u8>>,
    /// Actor NBT payloads referenced by the column's `digp`.
    pub actors: Vec<Vec<u8>>,
}

/// What [`BedrockWorld::delete_columns`] removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeleteStats {
    /// Distinct chunk columns that had at least one record removed.
    pub columns: usize,
    /// Chunk records + subchunks + digp keys removed.
    pub records: usize,
    /// `actorprefix` entity records removed (referenced by deleted digps).
    pub actors: usize,
}

impl BedrockWorld {
    /// Delete whole chunk columns of one dimension in a single pass over the
    /// database: every chunk record / subchunk whose *decoded* key matches
    /// (never a raw prefix match — an overworld base is a byte prefix of the
    /// nether/end keys at the same x,z), each column's `digp`, and the
    /// `actorprefix` records that `digp` lists (pruning removes entities
    /// with their chunk, as UMT does).
    pub fn delete_columns(
        &self,
        dim: u8,
        columns: &HashSet<(i32, i32)>,
    ) -> WorldResult<DeleteStats> {
        if columns.is_empty() {
            return Ok(DeleteStats::default());
        }
        // Key-only discovery (never the whole database in memory): decoded
        // keys are matched, never a raw prefix (an overworld base is a byte
        // prefix of same-x,z nether/end keys).
        let matches = |k: &[u8]| -> Option<(i32, i32, bool)> {
            match keys::decode_key(k) {
                DbKey::ChunkRecord { x, z, dim: d, .. } | DbKey::Subchunk { x, z, dim: d, .. }
                    if d == dim && columns.contains(&(x, z)) =>
                {
                    Some((x, z, false))
                }
                DbKey::Digp { x, z, dim: d } if d == dim && columns.contains(&(x, z)) => Some((x, z, true)),
                _ => None,
            }
        };
        let mut found: Vec<Vec<u8>> = if columns.len() <= 64 {
            // Few columns: targeted prefix key scans.
            let mut v = Vec::new();
            for &(x, z) in columns {
                for prefix in [keys::chunk_base(x, z, dim), keys::digp_key(x, z, dim)] {
                    let ks = self
                        .db()
                        .collect_prefix_keys_owned(&prefix, Self::cached_reads())
                        .map_err(|e| WorldError::Store(format!("leveldb prefix scan: {e}")))?;
                    v.extend(ks.into_iter().filter(|k| matches(k).is_some()).map(|k| k.to_vec()));
                }
            }
            v
        } else {
            let (_, parts) = self
                .db()
                .for_each_key_partitioned(Self::parallel_scan(), Vec::new, |v: &mut Vec<Vec<u8>>, key| {
                    if matches(key).is_some() {
                        v.push(key.to_vec());
                    }
                    Ok(VisitorControl::Continue)
                })
                .map_err(|e| WorldError::Store(format!("leveldb key scan: {e}")))?;
            parts.into_iter().flatten().collect()
        };
        found.sort_unstable();
        found.dedup();
        // Keys from older tables may be shadowed by newer tombstones: keep
        // only live ones (and read digp values for their actor ids).
        let vals = self.get_exact(&found)?;
        let mut victims: Vec<Vec<u8>> = Vec::new();
        let mut touched = HashSet::new();
        let mut stats = DeleteStats::default();
        for (k, v) in found.into_iter().zip(vals) {
            let Some(v) = v else { continue };
            let Some((x, z, digp)) = matches(&k) else { continue };
            if digp {
                for id in v.as_chunks::<8>().0 {
                    victims.push(keys::actor_key(*id));
                    stats.actors += 1;
                }
            }
            touched.insert((x, z));
            victims.push(k);
            stats.records += 1;
        }
        stats.columns = touched.len();
        let db = self.db();
        for chunk in victims.chunks(20_000) {
            let mut batch = WriteBatch::new();
            for k in chunk {
                batch.delete(k.clone());
            }
            db.write(batch, WriteOptions::default())
                .map_err(|e| WorldError::Store(format!("leveldb write: {e}")))?;
        }
        Ok(stats)
    }

    /// Full database compaction (UMT's pruner "Optimize" tool): rewrites the
    /// tables so deleted keys stop occupying disk space.
    pub fn compact(&self) -> WorldResult<()> {
        let db = self.db();
        db.flush().map_err(|e| WorldError::Store(format!("leveldb flush: {e}")))?;
        db.compact_range(None, None)
            .map_err(|e| WorldError::Store(format!("leveldb compact: {e}")))
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
        let mut one = std::collections::HashSet::new();
        one.insert((x, z));
        self.delete_columns(dim, &one).map(|_| ())
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
