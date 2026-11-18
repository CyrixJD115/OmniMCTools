//! On-disk tile cache ("pixel cache"): rendered tiles are written as small
//! zlib-compressed files keyed by **content** — the source fingerprint of
//! the tile's area (Bedrock manifest / Java region headers), the render
//! options, the dimension, the LOD level and the sampling stride. Reopening
//! a world (even a fresh workspace copy) reuses every tile whose data did
//! not change; any edit changes the fingerprint, so stale pixels are never
//! shown. The cache is size-capped and pruned oldest-first.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::area::{AreaStats, TileImage, TileKey};
use crate::render::RenderOptions;
use crate::source::Fnv;

/// Bump when rendering output changes (colors, shading, LOD math).
pub const RENDER_VERSION: u32 = 4;
const MAGIC: &[u8; 4] = b"OMTT";

/// Stable cache key of a tile.
pub fn tile_key_hash(fingerprint: u64, dim: u8, opts: &RenderOptions, key: TileKey, stride: i32) -> u64 {
    let mut h = Fnv::new();
    h.u64(RENDER_VERSION as u64);
    h.u64(fingerprint);
    h.u64(dim as u64);
    h.u64(opts.max_y.map_or(u64::MAX, |y| y as i64 as u64));
    h.u64(opts.min_y as i64 as u64);
    h.u64(opts.biome_tint as u64 | (opts.water_shading as u64) << 1 | (opts.relief as u64) << 2 | (opts.cave as u64) << 3);
    h.u64(key.level as u64);
    h.u64(key.tx as i64 as u64);
    h.u64(key.tz as i64 as u64);
    h.u64(stride as i64 as u64);
    h.finish()
}

#[derive(Debug, Clone)]
pub struct TileCache {
    dir: PathBuf,
    max_bytes: u64,
}

impl TileCache {
    pub fn new(dir: impl Into<PathBuf>, max_bytes: u64) -> Self {
        Self { dir: dir.into(), max_bytes }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, hash: u64) -> PathBuf {
        self.dir.join(format!("{:02x}", hash >> 56)).join(format!("{hash:016x}.omtt"))
    }

    /// Load a cached tile (any read/format error = miss).
    pub fn load(&self, hash: u64, key: TileKey) -> Option<TileImage> {
        let bytes = std::fs::read(self.path(hash)).ok()?;
        let t = decode(&bytes, key)?;
        // Touch for LRU pruning (best effort).
        if let Ok(f) = std::fs::File::options().append(true).open(self.path(hash)) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        Some(t)
    }

    /// Store a tile (best effort; errors are ignored — a cache never fails
    /// a render). Written atomically via a temp file.
    pub fn store(&self, hash: u64, tile: &TileImage) {
        let path = self.path(hash);
        let Some(dir) = path.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(&tmp, encode(tile)).is_ok() && std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// Total bytes and file count on disk.
    pub fn usage(&self) -> (u64, usize) {
        let mut total = (0u64, 0usize);
        for (_, len, _) in self.files() {
            total.0 += len;
            total.1 += 1;
        }
        total
    }

    fn files(&self) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
        let mut out = Vec::new();
        let Ok(dirs) = std::fs::read_dir(&self.dir) else { return out };
        for d in dirs.flatten() {
            let Ok(entries) = std::fs::read_dir(d.path()) else { continue };
            for e in entries.flatten() {
                if let Ok(m) = e.metadata()
                    && m.is_file()
                {
                    out.push((e.path(), m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH)));
                }
            }
        }
        out
    }

    /// Delete oldest files until the cache fits its cap. Returns bytes freed.
    pub fn prune(&self) -> u64 {
        let mut files = self.files();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= self.max_bytes {
            return 0;
        }
        files.sort_by_key(|f| f.2);
        let mut freed = 0;
        for (p, len, _) in files {
            if total <= self.max_bytes * 9 / 10 {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                total -= len;
                freed += len;
            }
        }
        freed
    }

    /// Remove every cached tile.
    pub fn clear(&self) -> std::io::Result<()> {
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

fn encode(t: &TileImage) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + t.rgba.len() / 4);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&RENDER_VERSION.to_le_bytes());
    out.push(t.key.level);
    out.extend_from_slice(&t.key.tx.to_le_bytes());
    out.extend_from_slice(&t.key.tz.to_le_bytes());
    out.extend_from_slice(&t.stride.to_le_bytes());
    out.extend_from_slice(&t.px.to_le_bytes());
    out.extend_from_slice(&(t.stats.chunks as u32).to_le_bytes());
    out.extend_from_slice(&(t.stats.errors as u32).to_le_bytes());
    out.extend_from_slice(&(t.names.len() as u32).to_le_bytes());
    for n in &t.names {
        out.extend_from_slice(&(n.len() as u16).to_le_bytes());
        out.extend_from_slice(n.as_bytes());
    }
    let mut payload = Vec::with_capacity(t.rgba.len() * 2);
    payload.extend_from_slice(&t.rgba);
    for s in &t.surface {
        payload.extend_from_slice(&s.to_le_bytes());
    }
    for h in &t.heights {
        payload.extend_from_slice(&h.to_le_bytes());
    }
    for b in &t.biomes {
        payload.extend_from_slice(&b.to_le_bytes());
    }
    let mut enc = flate2::write::ZlibEncoder::new(out, flate2::Compression::fast());
    let _ = enc.write_all(&payload);
    enc.finish().unwrap_or_default()
}

fn decode(bytes: &[u8], key: TileKey) -> Option<TileImage> {
    let mut c = bytes;
    let take = |c: &mut &[u8], n: usize| -> Option<Vec<u8>> {
        if c.len() < n {
            return None;
        }
        let (a, b) = c.split_at(n);
        *c = b;
        Some(a.to_vec())
    };
    let u32le = |c: &mut &[u8]| -> Option<u32> { Some(u32::from_le_bytes(take(c, 4)?.try_into().ok()?)) };
    if take(&mut c, 4)? != MAGIC || u32le(&mut c)? != RENDER_VERSION {
        return None;
    }
    let level = take(&mut c, 1)?[0];
    let tx = u32le(&mut c)? as i32;
    let tz = u32le(&mut c)? as i32;
    if TileKey::new(level, tx, tz) != key {
        return None;
    }
    let stride = u32le(&mut c)? as i32;
    let px = u32le(&mut c)?;
    let chunks = u32le(&mut c)? as usize;
    let errors = u32le(&mut c)? as usize;
    let n_names = u32le(&mut c)? as usize;
    let mut names = Vec::with_capacity(n_names.min(65_536));
    for _ in 0..n_names {
        let len = u16::from_le_bytes(take(&mut c, 2)?.try_into().ok()?) as usize;
        names.push(String::from_utf8(take(&mut c, len)?).ok()?);
    }
    let n = px as usize * px as usize;
    let mut payload = Vec::with_capacity(n * 10);
    flate2::read::ZlibDecoder::new(c).read_to_end(&mut payload).ok()?;
    if payload.len() != n * 10 {
        return None;
    }
    let rgba = payload[..n * 4].to_vec();
    let surface = payload[n * 4..n * 6].as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
    let heights = payload[n * 6..n * 8].as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    let biomes = payload[n * 8..].as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
    Some(TileImage { key, stride, px, rgba, surface, heights, biomes, names, stats: AreaStats { chunks, errors }, millis: 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_prune_and_key_sensitivity() {
        let dir = std::env::temp_dir().join(format!("omt-tilecache-test-{}", std::process::id()));
        let cache = TileCache::new(&dir, 1);
        let key = TileKey::new(1, -2, 3);
        let tile = TileImage {
            key,
            stride: 2,
            px: 4,
            rgba: (0..64).collect(),
            surface: vec![0, 1, u16::MAX, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            heights: (0..16).map(|i| i as i16 - 8).collect(),
            biomes: vec![2; 16],
            names: vec!["minecraft:stone".into(), "minecraft:grass_block".into(), "minecraft:plains".into()],
            stats: AreaStats { chunks: 7, errors: 1 },
            millis: 0,
        };
        let o = RenderOptions::default();
        let h = tile_key_hash(42, 0, &o, key, 2);
        assert_ne!(h, tile_key_hash(43, 0, &o, key, 2), "fingerprint changes the key");
        assert_ne!(h, tile_key_hash(42, 0, &RenderOptions { max_y: Some(60), ..o }, key, 2), "slicer changes the key");
        assert_ne!(h, tile_key_hash(42, 0, &o, key, 4), "stride changes the key");
        cache.store(h, &tile);
        let back = cache.load(h, key).expect("hit");
        assert_eq!((back.px, back.stride, &back.rgba, &back.surface, &back.heights, &back.names), (4, 2, &tile.rgba, &tile.surface, &tile.heights, &tile.names));
        assert_eq!(back.biomes, tile.biomes);
        assert_eq!(back.stats, tile.stats);
        assert!(cache.load(h, TileKey::new(1, -2, 4)).is_none(), "key mismatch is a miss");
        assert_eq!(cache.usage().1, 1);
        assert!(cache.prune() > 0, "1-byte cap prunes");
        assert!(cache.load(h, key).is_none());
        cache.clear().unwrap();
    }
}
