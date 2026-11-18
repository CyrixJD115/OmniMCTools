//! `omt-edit` — Amulet-style world editing in Rust: box selections and
//! block operations (fill, replace, clear, paste/clone, count) applied
//! directly to a world's stored chunk data, with undo, plus structure
//! export/import (`.mcstructure`, Java structure `.nbt`).
//!
//! Edits patch the *existing* records in place — Bedrock subchunk
//! payloads (byte-faithful codec; palette NBT incl. `version` preserved),
//! Java region sections (every other chunk copied sector-verbatim) — so
//! everything an operation does not touch (entities, ticks, biomes, other
//! sections) stays byte-identical. Block entities at changed positions are
//! removed (their block is gone). Java chunks are flagged `isLightOn = 0`
//! so the game relights them; Bedrock computes light at runtime. Chunks
//! that do not exist are never created (counted as `missing_chunks`).
//!
//! Always run against a workspace copy (the GUI does; Save writes back).

pub mod bedrock;
pub mod biome;
pub mod entities;
pub mod java;
pub mod structure;
pub mod transform;
pub mod verify;

use std::path::PathBuf;
use std::sync::Arc;

use omt_nbt::Tag;
use omt_world::BlockState;

/// An inclusive block-space box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sel {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl Sel {
    /// Box spanning two corners (any order).
    pub fn new(a: [i32; 3], b: [i32; 3]) -> Self {
        Self { min: [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])], max: [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])] }
    }

    pub fn size(&self) -> [i32; 3] {
        [self.max[0] - self.min[0] + 1, self.max[1] - self.min[1] + 1, self.max[2] - self.min[2] + 1]
    }

    pub fn volume(&self) -> u64 {
        let s = self.size();
        s[0] as u64 * s[1] as u64 * s[2] as u64
    }

    pub fn contains(&self, x: i32, y: i32, z: i32) -> bool {
        (self.min[0]..=self.max[0]).contains(&x) && (self.min[1]..=self.max[1]).contains(&y) && (self.min[2]..=self.max[2]).contains(&z)
    }

    /// Chunk columns the box touches.
    pub fn chunks(&self) -> impl Iterator<Item = (i32, i32)> {
        let (cx0, cz0, cx1, cz1) = (self.min[0].div_euclid(16), self.min[2].div_euclid(16), self.max[0].div_euclid(16), self.max[2].div_euclid(16));
        (cz0..=cz1).flat_map(move |cz| (cx0..=cx1).map(move |cx| (cx, cz)))
    }

    /// Clip to a Y range.
    pub fn clamp_y(mut self, lo: i32, hi: i32) -> Self {
        self.min[1] = self.min[1].max(lo);
        self.max[1] = self.max[1].min(hi);
        self
    }
}

/// A block plus (Bedrock) its palette NBT, so states and `version` are
/// written back exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub state: BlockState,
    pub bedrock_raw: Option<Tag>,
}

impl Block {
    pub fn named(name: &str) -> Self {
        let name = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
        Self { state: BlockState { name, properties: Vec::new() }, bedrock_raw: None }
    }

    pub fn air() -> Self {
        Self::named("minecraft:air")
    }

    pub fn is_air(&self) -> bool {
        is_air(&self.state)
    }
}

pub fn is_air(s: &BlockState) -> bool {
    matches!(s.name.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
}

/// Block matcher: name (namespace optional) and optional properties.
#[derive(Debug, Clone, PartialEq)]
pub struct Matcher {
    pub name: String,
    pub props: Vec<(String, String)>,
}

impl Matcher {
    /// Parse `name` or `name[key=value,…]`.
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        let (name, props) = match s.split_once('[') {
            Some((n, rest)) => {
                let props = rest.trim_end_matches(']').split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
                (n.trim(), props)
            }
            None => (s, Vec::new()),
        };
        let name = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
        Self { name, props }
    }

    pub fn matches(&self, s: &BlockState) -> bool {
        s.name == self.name && self.props.iter().all(|(k, v)| s.properties.iter().any(|(pk, pv)| pk == k && pv.trim_matches('"') == v))
    }
}

/// A dense box of blocks (copy buffer / structure), index
/// `(y * size_z + z) * size_x + x`.
#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    pub size: [i32; 3],
    pub palette: Vec<Block>,
    pub idx: Vec<u32>,
}

impl Volume {
    pub fn at(&self, x: i32, y: i32, z: i32) -> Option<&Block> {
        if x < 0 || y < 0 || z < 0 || x >= self.size[0] || y >= self.size[1] || z >= self.size[2] {
            return None;
        }
        let i = ((y * self.size[2] + z) * self.size[0] + x) as usize;
        self.palette.get(*self.idx.get(i)? as usize)
    }
}

/// An edit operation.
#[derive(Debug, Clone)]
pub enum Op {
    /// Set every block (or only air) in the box.
    Fill { to: Block, only_air: bool },
    /// Swap matching blocks.
    Replace { from: Matcher, to: Block },
    /// Set to air.
    Clear,
    /// Paste a volume with its minimum corner at `origin`.
    Paste { vol: Arc<Volume>, origin: [i32; 3], skip_air: bool },
}

impl Op {
    /// The block to write at a position currently holding `old`
    /// (`None` = leave it).
    pub fn decide<'a>(&'a self, x: i32, y: i32, z: i32, old: &BlockState, air: &'a Block) -> Option<&'a Block> {
        let new = match self {
            Op::Fill { to, only_air } => (!*only_air || is_air(old)).then_some(to),
            Op::Replace { from, to } => from.matches(old).then_some(to),
            Op::Clear => (!is_air(old)).then_some(air),
            Op::Paste { vol, origin, skip_air } => vol.at(x - origin[0], y - origin[1], z - origin[2]).filter(|b| !(*skip_air && b.is_air())),
        }?;
        (new.state != *old).then_some(new)
    }

    /// The box this op writes into, given the selection.
    pub fn target(&self, sel: Sel) -> Sel {
        match self {
            Op::Paste { vol, origin, .. } => Sel::new(*origin, [origin[0] + vol.size[0] - 1, origin[1] + vol.size[1] - 1, origin[2] + vol.size[2] - 1]),
            _ => sel,
        }
    }
}

/// What an operation did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub changed: u64,
    pub chunks: usize,
    pub missing_chunks: usize,
    pub block_entities_removed: usize,
    /// Block counts (read pass).
    pub counts: Vec<(String, u64)>,
}

/// Previous record contents, enough to revert an operation.
#[derive(Debug, Clone)]
pub enum Undo {
    /// LevelDB key → previous value (`None` = key did not exist).
    Bedrock(Vec<(Vec<u8>, Option<Vec<u8>>)>),
    /// Region file → previous bytes.
    Java(Vec<(PathBuf, Vec<u8>)>),
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("world: {0}")]
    World(#[from] omt_world::WorldError),
    #[error("corrupt data: {0}")]
    Corrupt(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, EditError>;

/// An editable world.
#[derive(Clone)]
pub enum EditWorld {
    Java(Arc<omt_java::JavaWorld>),
    Bedrock(Arc<omt_bedrock::BedrockWorld>),
}

/// World Y range per dimension (current versions).
pub fn y_range(dim: u8) -> (i32, i32) {
    match dim {
        1 => (0, 127),
        2 => (0, 255),
        _ => (-64, 319),
    }
}

/// Apply `op` inside `sel` (writes go to the store immediately).
pub fn apply(w: &EditWorld, dim: u8, sel: Sel, op: &Op, cancel: &(dyn Fn() -> bool + Sync)) -> Result<(Report, Undo)> {
    let (lo, hi) = y_range(dim);
    let target = op.target(sel).clamp_y(lo, hi);
    if target.min[1] > target.max[1] {
        return Ok((Report::default(), match w {
            EditWorld::Java(_) => Undo::Java(Vec::new()),
            EditWorld::Bedrock(_) => Undo::Bedrock(Vec::new()),
        }));
    }
    match w {
        EditWorld::Bedrock(b) => bedrock::apply(b, dim, target, op, cancel),
        EditWorld::Java(j) => java::apply(j, dim, target, op, cancel),
    }
}

/// Read a box into a volume (copy / export) and count its blocks.
pub fn read(w: &EditWorld, dim: u8, sel: Sel, cancel: &(dyn Fn() -> bool + Sync)) -> Result<(Volume, Report)> {
    let (lo, hi) = y_range(dim);
    let sel = sel.clamp_y(lo, hi);
    let vol = match w {
        EditWorld::Bedrock(b) => bedrock::read(b, dim, sel, cancel)?,
        EditWorld::Java(j) => java::read(j, dim, sel, cancel)?,
    };
    let mut counts: std::collections::HashMap<&str, u64> = std::collections::HashMap::new();
    let mut per_entry = vec![0u64; vol.palette.len()];
    for &i in &vol.idx {
        per_entry[i as usize] += 1;
    }
    for (b, n) in vol.palette.iter().zip(per_entry) {
        if n > 0 {
            *counts.entry(b.state.name.as_str()).or_default() += n;
        }
    }
    let mut counts: Vec<(String, u64)> = counts.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let report = Report { counts, chunks: sel.chunks().count(), ..Default::default() };
    Ok((vol, report))
}

/// Revert an operation.
pub fn undo(w: &EditWorld, u: &Undo) -> Result<()> {
    match (w, u) {
        (EditWorld::Bedrock(b), Undo::Bedrock(kv)) => {
            let puts: Vec<(Vec<u8>, Vec<u8>)> = kv.iter().filter_map(|(k, v)| v.clone().map(|v| (k.clone(), v))).collect();
            let dels: Vec<Vec<u8>> = kv.iter().filter(|(_, v)| v.is_none()).map(|(k, _)| k.clone()).collect();
            b.raw_put_many(&puts)?;
            b.raw_delete_many(&dels)?;
            Ok(())
        }
        (EditWorld::Java(_), Undo::Java(files)) => {
            for (path, bytes) in files {
                java::write_atomic(path, bytes)?;
            }
            Ok(())
        }
        _ => Err(EditError::Other("undo data belongs to the other edition".into())),
    }
}

/// Builder for volumes (shared by the edition readers).
pub(crate) struct VolumeBuilder {
    vol: Volume,
    ids: std::collections::HashMap<(String, Vec<(String, String)>), u32>,
}

impl VolumeBuilder {
    pub fn new(size: [i32; 3]) -> Self {
        let n = size[0] as usize * size[1] as usize * size[2] as usize;
        let mut b = Self { vol: Volume { size, palette: Vec::new(), idx: vec![0; n] }, ids: Default::default() };
        b.id(&Block::air());
        b
    }

    pub fn id(&mut self, b: &Block) -> u32 {
        let key = (b.state.name.clone(), b.state.properties.clone());
        if let Some(&i) = self.ids.get(&key) {
            return i;
        }
        self.vol.palette.push(b.clone());
        let i = (self.vol.palette.len() - 1) as u32;
        self.ids.insert(key, i);
        i
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, id: u32) {
        let s = self.vol.size;
        let i = ((y * s[2] + z) * s[0] + x) as usize;
        self.vol.idx[i] = id;
    }

    pub fn finish(self) -> Volume {
        self.vol
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_geometry() {
        let s = Sel::new([5, 70, -3], [-20, 60, 17]);
        assert_eq!(s.min, [-20, 60, -3]);
        assert_eq!(s.size(), [26, 11, 21]);
        assert_eq!(s.volume(), 26 * 11 * 21);
        let c: Vec<_> = s.chunks().collect();
        assert_eq!(c.len(), 3 * 3, "x chunks -2..=0, z chunks -1..=1");
        assert!(c.contains(&(-2, -1)) && c.contains(&(0, 1)));
        assert_eq!(s.clamp_y(64, 300).min[1], 64);
    }

    #[test]
    fn matcher_and_decide() {
        let m = Matcher::parse("oak_log[axis=y]");
        assert_eq!(m.name, "minecraft:oak_log");
        let log_y = BlockState { name: "minecraft:oak_log".into(), properties: vec![("axis".into(), "y".into())] };
        let log_x = BlockState { name: "minecraft:oak_log".into(), properties: vec![("axis".into(), "x".into())] };
        assert!(m.matches(&log_y) && !m.matches(&log_x));
        assert!(Matcher::parse("minecraft:oak_log").matches(&log_x));
        let air = Block::air();
        let stone = Block::named("stone");
        let fill = Op::Fill { to: stone.clone(), only_air: true };
        assert!(fill.decide(0, 0, 0, &log_y, &air).is_none(), "only air");
        assert_eq!(fill.decide(0, 0, 0, &air.state, &air), Some(&stone));
        assert!(Op::Clear.decide(0, 0, 0, &air.state, &air).is_none(), "no-op skipped");
        let mut vb = VolumeBuilder::new([2, 1, 1]);
        let id = vb.id(&stone);
        vb.set(1, 0, 0, id);
        let vol = Arc::new(vb.finish());
        let paste = Op::Paste { vol, origin: [10, 64, 10], skip_air: true };
        assert_eq!(paste.decide(11, 64, 10, &log_y, &air), Some(&stone));
        assert!(paste.decide(10, 64, 10, &log_y, &air).is_none(), "air skipped");
        assert_eq!(paste.target(Sel::new([0; 3], [0; 3])).max, [11, 64, 10]);
    }
}
