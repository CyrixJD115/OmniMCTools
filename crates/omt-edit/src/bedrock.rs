//! Bedrock edits: subchunk payloads are decoded with the byte-faithful
//! codec, block storage 0 is patched (palette NBT kept in sync, unused
//! entries compacted, legal bit widths), re-encoded and written back in
//! one LevelDB batch per chunk. Waterlogging (storage 1) is untouched.

use std::collections::{HashMap, HashSet};

use omt_bedrock::BedrockWorld;
use omt_bedrock::keys::{self, tag};
use omt_bedrock::subchunk::{Subchunk, SubchunkStorage, block_index_xzy};
use omt_nbt::{ByteOrder, Nbt, Tag};
#[cfg(test)]
use omt_world::BlockState;

use crate::{Block, EditError, Op, Report, Result, Sel, Undo, Volume, VolumeBuilder};

type StateKey = (String, Vec<(String, String)>);

/// Bit widths Bedrock accepts for palette indices.
fn legal_bits(palette_len: usize) -> u8 {
    let need = (usize::BITS - palette_len.saturating_sub(1).leading_zeros()).max(1) as u8;
    [1u8, 2, 3, 4, 5, 6, 8, 16].into_iter().find(|&b| b >= need).unwrap_or(16)
}

/// A palette entry's NBT for a block: its own, a template seen in the
/// world, or built from name + states (+ the world's block `version`).
fn raw_entry(b: &Block, templates: &HashMap<StateKey, Tag>, version: Option<i32>) -> Tag {
    if let Some(r) = &b.bedrock_raw {
        return r.clone();
    }
    if let Some(t) = templates.get(&(b.state.name.clone(), b.state.properties.clone())) {
        return t.clone();
    }
    let states: Vec<(String, Tag)> = b
        .state
        .properties
        .iter()
        .map(|(k, v)| {
            let t = match omt_nbt::snbt::from_snbt(v) {
                Ok(t @ (Tag::Byte(_) | Tag::Short(_) | Tag::Int(_) | Tag::Long(_))) => t,
                _ => Tag::String(v.trim_matches('"').to_string()),
            };
            (k.clone(), t)
        })
        .collect();
    let mut fields = vec![("name".to_string(), Tag::String(b.state.name.clone())), ("states".to_string(), Tag::Compound(states))];
    if let Some(v) = version {
        fields.push(("version".to_string(), Tag::Int(v)));
    }
    Tag::Compound(fields)
}

fn harvest(st: &SubchunkStorage, templates: &mut HashMap<StateKey, Tag>, version: &mut Option<i32>) {
    if let Some(raw) = &st.raw_entries {
        for (s, r) in st.palette.iter().zip(raw) {
            if version.is_none()
                && let Some(Tag::Int(v)) = r.get("version")
            {
                *version = Some(*v);
            }
            templates.entry((s.name.clone(), s.properties.clone())).or_insert_with(|| r.clone());
        }
    }
}

fn ensure(st: &mut SubchunkStorage, b: &Block, templates: &HashMap<StateKey, Tag>, version: Option<i32>) -> u16 {
    if let Some(i) = st.palette.iter().position(|s| *s == b.state) {
        return i as u16;
    }
    st.palette.push(b.state.clone());
    if let Some(raw) = &mut st.raw_entries {
        raw.push(raw_entry(b, templates, version));
    }
    (st.palette.len() - 1) as u16
}

/// Drop unused palette entries and set canonical indices / bits.
fn compact(st: &mut SubchunkStorage, idx: Vec<u16>) {
    let mut used = vec![false; st.palette.len()];
    for &i in &idx {
        used[i as usize] = true;
    }
    let mut remap = vec![0u16; st.palette.len()];
    let mut palette = Vec::new();
    let mut raw = st.raw_entries.as_ref().map(|_| Vec::new());
    for (i, u) in used.iter().enumerate() {
        if *u {
            remap[i] = palette.len() as u16;
            palette.push(st.palette[i].clone());
            if let (Some(out), Some(src)) = (&mut raw, &st.raw_entries) {
                out.push(src[i].clone());
            }
        }
    }
    st.palette = palette;
    st.raw_entries = raw;
    if st.palette.len() <= 1 {
        st.indices = None;
        st.bits = 0;
    } else {
        st.indices = Some(idx.into_iter().map(|i| remap[i as usize]).collect());
        st.bits = legal_bits(st.palette.len());
    }
}

fn air_storage(version: Option<i32>) -> SubchunkStorage {
    let air = Block::air();
    let raw = raw_entry(&air, &HashMap::new(), version);
    SubchunkStorage { palette: vec![air.state], raw_entries: Some(vec![raw]), indices: None, bits: 0 }
}

fn corrupt(e: impl std::fmt::Display) -> EditError {
    EditError::Corrupt(e.to_string())
}

/// Concatenated little-endian NBT roots (block entity records).
pub fn parse_roots(raw: &[u8]) -> Result<Vec<Nbt>> {
    let mut out = Vec::new();
    let mut off = 0;
    while off < raw.len() {
        let (n, used) = omt_nbt::read_prefix(&raw[off..], ByteOrder::Little).map_err(corrupt)?;
        if used == 0 {
            break;
        }
        out.push(n);
        off += used;
    }
    Ok(out)
}

fn pos_of(t: &Tag) -> Option<(i32, i32, i32)> {
    let g = |k: &str| t.get(k).and_then(Tag::as_i64).map(|v| v as i32);
    Some((g("x")?, g("y")?, g("z")?))
}

pub fn apply(w: &BedrockWorld, dim: u8, sel: Sel, op: &Op, cancel: &(dyn Fn() -> bool + Sync)) -> Result<(Report, Undo)> {
    let air = Block::air();
    let mut report = Report::default();
    let mut undo: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    let mut templates: HashMap<StateKey, Tag> = HashMap::new();
    let mut version = None;
    for (cx, cz) in sel.chunks() {
        if cancel() {
            break;
        }
        if w.raw_get(&keys::version_key(cx, cz, dim))?.is_none() {
            report.missing_chunks += 1;
            continue;
        }
        let mut puts: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut changed: HashSet<(i32, i32, i32)> = HashSet::new();
        let (x0, x1) = (sel.min[0].max(cx * 16), sel.max[0].min(cx * 16 + 15));
        let (z0, z1) = (sel.min[2].max(cz * 16), sel.max[2].min(cz * 16 + 15));
        for sy in sel.min[1].div_euclid(16)..=sel.max[1].div_euclid(16) {
            let key = keys::subchunk_key(cx, cz, dim, sy as i8);
            let old = w.raw_get(&key)?;
            let mut sub = match &old {
                Some(b) => Subchunk::parse(b, Some(sy as i8)).map_err(corrupt)?,
                None => Subchunk { version: 9, y: sy as i8, storages: Vec::new() },
            };
            if sub.storages.is_empty() {
                sub.storages.push(air_storage(version));
            }
            for st in &sub.storages {
                harvest(st, &mut templates, &mut version);
            }
            let st = &mut sub.storages[0];
            let mut idx = st.indices.clone().unwrap_or_else(|| vec![0; 4096]);
            let mut decided: HashMap<u16, Option<u16>> = HashMap::new();
            let mut n = 0u64;
            let (y0, y1) = (sel.min[1].max(sy * 16), sel.max[1].min(sy * 16 + 15));
            for x in x0..=x1 {
                for z in z0..=z1 {
                    for y in y0..=y1 {
                        let i = block_index_xzy((x - cx * 16) as usize, (y - sy * 16) as usize, (z - cz * 16) as usize);
                        let cur = idx[i];
                        let new = match op {
                            Op::Paste { .. } => {
                                let b = op.decide(x, y, z, &st.palette[cur as usize], &air).cloned();
                                b.map(|b| ensure(st, &b, &templates, version))
                            }
                            _ => match decided.get(&cur) {
                                Some(d) => *d,
                                None => {
                                    let b = op.decide(x, y, z, &st.palette[cur as usize], &air).cloned();
                                    let d = b.map(|b| ensure(st, &b, &templates, version));
                                    decided.insert(cur, d);
                                    d
                                }
                            },
                        };
                        if let Some(p) = new
                            && p != cur
                        {
                            idx[i] = p;
                            n += 1;
                            changed.insert((x, y, z));
                        }
                    }
                }
            }
            if n > 0 {
                compact(st, idx);
                puts.push((key.clone(), sub.serialize()));
                undo.push((key, old));
                report.changed += n;
            }
        }
        if changed.is_empty() {
            continue;
        }
        report.chunks += 1;
        let be_key = keys::chunk_record(cx, cz, dim, tag::BLOCK_ENTITIES);
        if let Some(raw) = w.raw_get(&be_key)? {
            let roots = parse_roots(&raw)?;
            let before = roots.len();
            let keep: Vec<Nbt> = roots.into_iter().filter(|n| pos_of(&n.tag).is_none_or(|p| !changed.contains(&p))).collect();
            if keep.len() != before {
                report.block_entities_removed += before - keep.len();
                undo.push((be_key.clone(), Some(raw)));
                if keep.is_empty() {
                    w.raw_delete_many(&[be_key])?;
                } else {
                    let mut out = Vec::new();
                    for n in &keep {
                        out.extend(omt_nbt::write(n, ByteOrder::Little).map_err(corrupt)?);
                    }
                    puts.push((be_key, out));
                }
            }
        }
        w.raw_put_many(&puts)?;
    }
    Ok((report, Undo::Bedrock(undo)))
}

pub fn read(w: &BedrockWorld, dim: u8, sel: Sel, cancel: &(dyn Fn() -> bool + Sync)) -> Result<Volume> {
    let mut vb = VolumeBuilder::new(sel.size());
    for (cx, cz) in sel.chunks() {
        if cancel() {
            return Err(EditError::Cancelled);
        }
        let (x0, x1) = (sel.min[0].max(cx * 16), sel.max[0].min(cx * 16 + 15));
        let (z0, z1) = (sel.min[2].max(cz * 16), sel.max[2].min(cz * 16 + 15));
        for sy in sel.min[1].div_euclid(16)..=sel.max[1].div_euclid(16) {
            let Some(raw) = w.raw_get(&keys::subchunk_key(cx, cz, dim, sy as i8))? else { continue };
            let sub = Subchunk::parse(&raw, Some(sy as i8)).map_err(corrupt)?;
            let Some(st) = sub.storages.first() else { continue };
            let mut ids: Vec<Option<u32>> = vec![None; st.palette.len()];
            let (y0, y1) = (sel.min[1].max(sy * 16), sel.max[1].min(sy * 16 + 15));
            for x in x0..=x1 {
                for z in z0..=z1 {
                    for y in y0..=y1 {
                        let p = st.indices.as_ref().map_or(0, |ix| ix[block_index_xzy((x - cx * 16) as usize, (y - sy * 16) as usize, (z - cz * 16) as usize)]) as usize;
                        let Some(state) = st.palette.get(p) else { continue };
                        let id = match ids[p] {
                            Some(id) => id,
                            None => {
                                let b = Block { state: state.clone(), bedrock_raw: st.raw_entries.as_ref().and_then(|r| r.get(p).cloned()) };
                                let id = vb.id(&b);
                                ids[p] = Some(id);
                                id
                            }
                        };
                        vb.set(x - sel.min[0], y - sel.min[1], z - sel.min[2], id);
                    }
                }
            }
        }
    }
    Ok(vb.finish())
}

/// Palette NBT for a block (structure export).
pub(crate) fn raw_entry_pub(b: &Block, version: Option<i32>) -> Tag {
    raw_entry(b, &HashMap::new(), version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_and_compaction() {
        assert_eq!(legal_bits(2), 1);
        assert_eq!(legal_bits(5), 3);
        assert_eq!(legal_bits(100), 8, "7 is not a legal width");
        assert_eq!(legal_bits(70_000), 16);
        let mut st = air_storage(Some(18_100_737));
        let stone = Block::named("stone");
        let id = ensure(&mut st, &stone, &HashMap::new(), Some(18_100_737));
        assert_eq!(id, 1);
        assert_eq!(st.raw_entries.as_ref().unwrap()[1].get("version"), Some(&Tag::Int(18_100_737)));
        let mut idx = vec![0u16; 4096];
        idx[7] = 1;
        compact(&mut st, idx);
        assert_eq!(st.palette.len(), 2);
        assert_eq!(st.bits, 1);
        // All stone → single-entry canonical storage.
        compact(&mut st, vec![1; 4096]);
        assert_eq!((st.palette.len(), st.indices.is_none(), st.bits), (1, true, 0));
        assert_eq!(st.palette[0].name, "minecraft:stone");
        // Serialize → parse round trip keeps the edit.
        let sub = Subchunk { version: 9, y: 4, storages: vec![st] };
        let back = Subchunk::parse(&sub.serialize(), Some(4)).unwrap();
        assert_eq!(back.block(3, 3, 3).unwrap().name, "minecraft:stone");
    }

    #[test]
    fn typed_states_are_rebuilt() {
        let b = Block { state: BlockState { name: "minecraft:oak_log".into(), properties: vec![("pillar_axis".into(), "y".into()), ("age".into(), "3".into())] }, bedrock_raw: None };
        let t = raw_entry(&b, &HashMap::new(), None);
        let states = t.get("states").unwrap();
        assert_eq!(states.get("pillar_axis"), Some(&Tag::String("y".into())));
        assert_eq!(states.get("age"), Some(&Tag::Int(3)));
    }
}
