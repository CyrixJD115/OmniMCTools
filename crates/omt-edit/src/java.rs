//! Java edits: chunks are patched inside their region file (sections'
//! `block_states` palette + packed data; every other chunk copied
//! sector-verbatim), block entities at changed positions dropped, and the
//! chunk flagged `isLightOn = 0` so the game recomputes its light.
//! Current (1.18+) chunk layout only.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use omt_java::JavaWorld;
use omt_java::region::RegionFile;
use omt_nbt::{ByteOrder, Tag};
use omt_world::BlockState;
use omt_world::packing::{pack_java_indices, unpack_java_indices};

use crate::{Block, EditError, Op, Report, Result, Sel, Undo, Volume, VolumeBuilder};

fn corrupt(e: impl std::fmt::Display) -> EditError {
    EditError::Corrupt(e.to_string())
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("omt-tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn state_tag(s: &BlockState) -> Tag {
    let mut f = vec![("Name".to_string(), Tag::String(s.name.clone()))];
    if !s.properties.is_empty() {
        f.push(("Properties".to_string(), Tag::Compound(s.properties.iter().map(|(k, v)| (k.clone(), Tag::String(v.clone()))).collect())));
    }
    Tag::Compound(f)
}

/// Section palette + indices (YZX order) of a section compound.
fn section_blocks(sec: &Tag) -> (Vec<BlockState>, Vec<u16>) {
    let bs = sec.get("block_states");
    let palette: Vec<BlockState> = bs.and_then(|b| b.get("palette")).and_then(Tag::as_list).map(|l| l.iter().filter_map(omt_java::store::java_palette_entry).collect()).unwrap_or_default();
    let palette = if palette.is_empty() { vec![Block::air().state] } else { palette };
    let idx = match bs.and_then(|b| b.get("data")) {
        Some(Tag::LongArray(d)) if palette.len() > 1 => unpack_java_indices(d, palette.len()),
        _ => vec![0; 4096],
    };
    (palette, idx)
}

fn set_section_blocks(sec: &mut Tag, palette: &[BlockState], idx: &[u16]) {
    // Compact unused entries.
    let mut used = vec![false; palette.len()];
    for &i in idx {
        used[i as usize] = true;
    }
    let mut remap = vec![0u16; palette.len()];
    let mut pal = Vec::new();
    for (i, u) in used.iter().enumerate() {
        if *u {
            remap[i] = pal.len() as u16;
            pal.push(palette[i].clone());
        }
    }
    let mut f = vec![("palette".to_string(), Tag::List(pal.iter().map(state_tag).collect()))];
    if pal.len() > 1 {
        let idx: Vec<u16> = idx.iter().map(|&i| remap[i as usize]).collect();
        f.push(("data".to_string(), Tag::LongArray(pack_java_indices(&idx, pal.len()))));
    }
    sec.insert("block_states", Tag::Compound(f));
}

fn sections_mut(chunk: &mut Tag) -> Result<&mut Vec<Tag>> {
    match chunk.get_mut("sections") {
        Some(Tag::List(l)) => Ok(l),
        _ => Err(EditError::Corrupt("chunk has no sections list (pre-1.18 layout?)".into())),
    }
}

fn section_y(s: &Tag) -> Option<i32> {
    s.get("Y").and_then(Tag::as_i64).map(|v| v as i32)
}

/// Edit one chunk compound. Returns (blocks changed, block entities removed).
pub fn edit_chunk(chunk: &mut Tag, cx: i32, cz: i32, sel: Sel, op: &Op) -> Result<(u64, usize)> {
    let air = Block::air();
    let (x0, x1) = (sel.min[0].max(cx * 16), sel.max[0].min(cx * 16 + 15));
    let (z0, z1) = (sel.min[2].max(cz * 16), sel.max[2].min(cz * 16 + 15));
    let mut changed: HashSet<(i32, i32, i32)> = HashSet::new();
    let mut total = 0u64;
    let sections = sections_mut(chunk)?;
    for sy in sel.min[1].div_euclid(16)..=sel.max[1].div_euclid(16) {
        let pos = sections.iter().position(|s| section_y(s) == Some(sy));
        let (mut palette, mut idx) = match pos {
            Some(p) => section_blocks(&sections[p]),
            None => (vec![air.state.clone()], vec![0; 4096]),
        };
        let mut decided: HashMap<u16, Option<u16>> = HashMap::new();
        let mut n = 0u64;
        let (y0, y1) = (sel.min[1].max(sy * 16), sel.max[1].min(sy * 16 + 15));
        for y in y0..=y1 {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    let i = ((y - sy * 16) * 256 + (z - cz * 16) * 16 + (x - cx * 16)) as usize;
                    let cur = idx[i];
                    let resolve = |b: Option<Block>, palette: &mut Vec<BlockState>| {
                        b.map(|b| match palette.iter().position(|s| *s == b.state) {
                            Some(p) => p as u16,
                            None => {
                                palette.push(b.state);
                                (palette.len() - 1) as u16
                            }
                        })
                    };
                    let new = match op {
                        Op::Paste { .. } => {
                            let b = op.decide(x, y, z, &palette[cur as usize], &air).cloned();
                            resolve(b, &mut palette)
                        }
                        _ => match decided.get(&cur) {
                            Some(d) => *d,
                            None => {
                                let b = op.decide(x, y, z, &palette[cur as usize], &air).cloned();
                                let d = resolve(b, &mut palette);
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
        if n == 0 {
            continue;
        }
        total += n;
        match pos {
            Some(p) => set_section_blocks(&mut sections[p], &palette, &idx),
            None => {
                // New section: biomes copied from the nearest section.
                let biomes = sections
                    .iter()
                    .filter_map(|s| Some(((section_y(s)? - sy).abs(), s.get("biomes")?.clone())))
                    .min_by_key(|(d, _)| *d)
                    .map(|(_, b)| b)
                    .unwrap_or_else(|| Tag::Compound(vec![("palette".into(), Tag::List(vec![Tag::String("minecraft:plains".into())]))]));
                let mut sec = Tag::Compound(vec![("Y".into(), Tag::Byte(sy as i8)), ("biomes".into(), biomes)]);
                set_section_blocks(&mut sec, &palette, &idx);
                sections.push(sec);
                sections.sort_by_key(|s| section_y(s).unwrap_or(i32::MIN));
            }
        }
    }
    if total == 0 {
        return Ok((0, 0));
    }
    let mut removed = 0;
    if let Some(Tag::List(bes)) = chunk.get_mut("block_entities") {
        let before = bes.len();
        bes.retain(|t| {
            let g = |k: &str| t.get(k).and_then(Tag::as_i64).map(|v| v as i32);
            !matches!((g("x"), g("y"), g("z")), (Some(x), Some(y), Some(z)) if changed.contains(&(x, y, z)))
        });
        removed = before - bes.len();
    }
    chunk.insert("isLightOn", Tag::Byte(0));
    Ok((total, removed))
}

fn by_region(sel: Sel) -> BTreeMap<(i32, i32), Vec<(i32, i32)>> {
    let mut m: BTreeMap<(i32, i32), Vec<(i32, i32)>> = BTreeMap::new();
    for (cx, cz) in sel.chunks() {
        m.entry((cx.div_euclid(32), cz.div_euclid(32))).or_default().push((cx, cz));
    }
    m
}

pub fn apply(w: &JavaWorld, dim: u8, sel: Sel, op: &Op, cancel: &(dyn Fn() -> bool + Sync)) -> Result<(Report, Undo)> {
    for_chunks(w, dim, sel, false, cancel, &mut |t, cx, cz| edit_chunk(t, cx, cz, sel, op))
}

/// Per-chunk edit: (chunk compound, cx, cz) → (blocks changed, block
/// entities removed).
pub(crate) type ChunkEdit<'a> = dyn FnMut(&mut Tag, i32, i32) -> Result<(u64, usize)> + 'a;

/// Run `edit` on every existing chunk the box touches, region by region
/// (each touched region rewritten once, atomically; previous bytes kept
/// for undo). `edit` returns (blocks changed, block entities removed).
pub(crate) fn for_chunks(
    w: &JavaWorld,
    dim: u8,
    sel: Sel,
    entities: bool,
    cancel: &(dyn Fn() -> bool + Sync),
    edit: &mut ChunkEdit<'_>,
) -> Result<(Report, Undo)> {
    let mut report = Report::default();
    let mut undo = Vec::new();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
    for ((rx, rz), chunks) in by_region(sel) {
        if cancel() {
            break;
        }
        let path = if entities {
            match w.entity_dimension_dir(dim) {
                Some(d) => d.join(format!("r.{rx}.{rz}.mca")),
                None => {
                    report.missing_chunks += chunks.len();
                    continue;
                }
            }
        } else {
            w.region_path(dim, rx, rz)
        };
        let Ok(orig) = std::fs::read(&path) else {
            report.missing_chunks += chunks.len();
            continue;
        };
        let mut bytes = orig.clone();
        let mut touched = false;
        for (cx, cz) in chunks {
            let region = RegionFile::from_bytes(&path, &bytes).map_err(corrupt)?;
            if !region.contains(cx, cz) {
                report.missing_chunks += 1;
                continue;
            }
            let mut nbt = region.read_chunk(&bytes, cx, cz).map_err(corrupt)?;
            let (n, be) = edit(&mut nbt.tag, cx, cz)?;
            if n > 0 {
                let raw = omt_nbt::write(&nbt, ByteOrder::Big).map_err(corrupt)?;
                bytes = region.with_chunk(&bytes, cx, cz, &raw, now);
                touched = true;
                report.changed += n;
                report.chunks += 1;
                report.block_entities_removed += be;
            }
        }
        if touched {
            write_atomic(&path, &bytes)?;
            undo.push((path, orig));
        }
    }
    Ok((report, Undo::Java(undo)))
}

pub fn read(w: &JavaWorld, dim: u8, sel: Sel, cancel: &(dyn Fn() -> bool + Sync)) -> Result<Volume> {
    let mut vb = VolumeBuilder::new(sel.size());
    for (cx, cz) in sel.chunks() {
        if cancel() {
            return Err(EditError::Cancelled);
        }
        let Some(nbt) = w.read_chunk_nbt(dim, cx, cz)? else { continue };
        let Some(Tag::List(sections)) = nbt.tag.get("sections") else { continue };
        let (x0, x1) = (sel.min[0].max(cx * 16), sel.max[0].min(cx * 16 + 15));
        let (z0, z1) = (sel.min[2].max(cz * 16), sel.max[2].min(cz * 16 + 15));
        for sec in sections {
            let Some(sy) = section_y(sec) else { continue };
            let (y0, y1) = (sel.min[1].max(sy * 16), sel.max[1].min(sy * 16 + 15));
            if y0 > y1 {
                continue;
            }
            let (palette, idx) = section_blocks(sec);
            let ids: Vec<u32> = palette.iter().map(|s| vb.id(&Block { state: s.clone(), bedrock_raw: None })).collect();
            for y in y0..=y1 {
                for z in z0..=z1 {
                    for x in x0..=x1 {
                        let i = ((y - sy * 16) * 256 + (z - cz * 16) * 16 + (x - cx * 16)) as usize;
                        vb.set(x - sel.min[0], y - sel.min[1], z - sel.min[2], ids[idx[i] as usize]);
                    }
                }
            }
        }
    }
    Ok(vb.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk() -> Tag {
        let stone = state_tag(&Block::named("stone").state);
        let air = state_tag(&Block::air().state);
        let sec0 = Tag::Compound(vec![
            ("Y".into(), Tag::Byte(4)),
            ("block_states".into(), Tag::Compound(vec![("palette".into(), Tag::List(vec![air, stone]))])),
            ("biomes".into(), Tag::Compound(vec![("palette".into(), Tag::List(vec![Tag::String("minecraft:forest".into())]))])),
        ]);
        // Palette [air, stone] without data = all air (index 0).
        let be = Tag::Compound(vec![("id".into(), Tag::String("minecraft:chest".into())), ("x".into(), Tag::Int(3)), ("y".into(), Tag::Int(70)), ("z".into(), Tag::Int(3))]);
        Tag::Compound(vec![("sections".into(), Tag::List(vec![sec0])), ("block_entities".into(), Tag::List(vec![be])), ("isLightOn".into(), Tag::Byte(1))])
    }

    #[test]
    fn fill_patches_sections_and_creates_new_ones() {
        let mut c = chunk();
        let op = Op::Fill { to: Block::named("gold_block"), only_air: false };
        // Box crosses section 4 (y 64..79) into 5 (80..95, absent).
        let (n, be) = edit_chunk(&mut c, 0, 0, Sel::new([0, 70, 0], [3, 81, 3]), &op).unwrap();
        assert_eq!(n, 4 * 12 * 4);
        assert_eq!(be, 1, "chest at (3,70,3) removed");
        assert_eq!(c.get("isLightOn"), Some(&Tag::Byte(0)));
        let secs = c.get("sections").and_then(Tag::as_list).unwrap();
        assert_eq!(secs.len(), 2);
        assert_eq!(section_y(&secs[1]), Some(5));
        assert_eq!(secs[1].get("biomes").and_then(|b| b.get("palette")).and_then(Tag::as_list).unwrap()[0], Tag::String("minecraft:forest".into()), "biomes copied");
        let (pal, idx) = section_blocks(&secs[0]);
        let at = |x: usize, y: usize, z: usize| &pal[idx[y * 256 + z * 16 + x] as usize].name;
        assert_eq!(at(0, 6, 0), "minecraft:gold_block");
        assert_eq!(at(4, 6, 0), "minecraft:air", "outside the box");
        assert!(!pal.iter().any(|s| s.name == "minecraft:stone"), "unused entries compacted");
        // Clear it again: back to air.
        let (n2, _) = edit_chunk(&mut c, 0, 0, Sel::new([0, 70, 0], [3, 81, 3]), &Op::Clear).unwrap();
        assert_eq!(n2, n);
        let secs = c.get("sections").and_then(Tag::as_list).unwrap();
        let (pal, _) = section_blocks(&secs[0]);
        assert_eq!(pal.len(), 1);
    }
}
