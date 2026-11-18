//! Biome painting: set the biome inside a box.
//! * Java: per-section `biomes` (4×4×4 cells, index `y<<4 | z<<2 | x`);
//!   every cell the box touches is painted (cell granularity is Java's).
//! * Bedrock: the `data3D` record's per-subchunk biome storages (per
//!   block, XZY order); `NonExistent` slots ("same as below") are
//!   materialized before painting. Biome names map to Bedrock ids through
//!   the converter registry.

use omt_bedrock::BedrockWorld;
use omt_bedrock::keys;
use omt_bedrock::subchunk::{Data3d, block_index_xzy};
use omt_nbt::Tag;
use omt_world::BedrockBiomeSlot;
use omt_world::packing::{pack_java_biome_cells, unpack_java_biome_cells};

use crate::{EditError, EditWorld, Report, Result, Sel, Undo};

fn norm(name: &str) -> String {
    if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") }
}

/// Bedrock numeric id of a biome name.
pub fn bedrock_biome_id(name: &str) -> Option<i32> {
    let ids = &omt_convert::RegistryRules::embedded().biome_ids;
    let n = norm(name);
    ids.get(&n).or_else(|| ids.get(n.trim_start_matches("minecraft:"))).copied()
}

/// Paint `biome` inside `sel`. Returns cells (Java) / blocks (Bedrock)
/// changed in `Report::changed`.
pub fn paint(w: &EditWorld, dim: u8, sel: Sel, biome: &str, cancel: &(dyn Fn() -> bool + Sync)) -> Result<(Report, Undo)> {
    let (lo, hi) = crate::y_range(dim);
    let sel = sel.clamp_y(lo, hi);
    match w {
        EditWorld::Java(j) => {
            let name = norm(biome);
            crate::java::for_chunks(j, dim, sel, false, cancel, &mut |t, cx, cz| Ok((paint_java_chunk(t, cx, cz, sel, &name), 0)))
        }
        EditWorld::Bedrock(b) => {
            let id = bedrock_biome_id(biome).ok_or_else(|| EditError::Other(format!("unknown biome {biome}")))?;
            paint_bedrock(b, dim, sel, id, lo, cancel)
        }
    }
}

/// Paint the Java biome cells of one chunk compound.
pub fn paint_java_chunk(chunk: &mut Tag, cx: i32, cz: i32, sel: Sel, biome: &str) -> u64 {
    let Some(Tag::List(sections)) = chunk.get_mut("sections") else { return 0 };
    let mut changed = 0;
    for sec in sections.iter_mut() {
        let Some(sy) = sec.get("Y").and_then(Tag::as_i64).map(|v| v as i32) else { continue };
        let (y0, y1) = (sel.min[1].max(sy * 16), sel.max[1].min(sy * 16 + 15));
        if y0 > y1 {
            continue;
        }
        let b = sec.get("biomes");
        let mut palette: Vec<String> = b.and_then(|b| b.get("palette")).and_then(Tag::as_list).map(|l| l.iter().filter_map(|t| t.as_str().map(str::to_string)).collect()).unwrap_or_default();
        if palette.is_empty() {
            palette.push("minecraft:plains".into());
        }
        let mut cells = match b.and_then(|b| b.get("data")) {
            Some(Tag::LongArray(d)) if palette.len() > 1 => unpack_java_biome_cells(d, palette.len()),
            _ => vec![0; 64],
        };
        let id = match palette.iter().position(|p| p == biome) {
            Some(i) => i as u16,
            None => {
                palette.push(biome.to_string());
                (palette.len() - 1) as u16
            }
        };
        let cell = |v: i32, base: i32| ((v - base).clamp(0, 15) / 4) as usize;
        let (x0, x1) = (sel.min[0].max(cx * 16), sel.max[0].min(cx * 16 + 15));
        let (z0, z1) = (sel.min[2].max(cz * 16), sel.max[2].min(cz * 16 + 15));
        if x0 > x1 || z0 > z1 {
            continue;
        }
        let mut n = 0;
        for cy in cell(y0, sy * 16)..=cell(y1, sy * 16) {
            for czz in cell(z0, cz * 16)..=cell(z1, cz * 16) {
                for cxx in cell(x0, cx * 16)..=cell(x1, cx * 16) {
                    let i = (cy << 4) | (czz << 2) | cxx;
                    if cells[i] != id {
                        cells[i] = id;
                        n += 1;
                    }
                }
            }
        }
        if n == 0 {
            continue;
        }
        changed += n;
        // Compact the palette.
        let mut used: Vec<u16> = cells.clone();
        used.sort_unstable();
        used.dedup();
        let new_pal: Vec<String> = used.iter().map(|&u| palette[u as usize].clone()).collect();
        let cells: Vec<u16> = cells.iter().map(|c| used.iter().position(|u| u == c).unwrap_or(0) as u16).collect();
        let mut f = vec![("palette".to_string(), Tag::List(new_pal.iter().map(|s| Tag::String(s.clone())).collect()))];
        if new_pal.len() > 1 {
            f.push(("data".to_string(), Tag::LongArray(pack_java_biome_cells(&cells, new_pal.len()))));
        }
        sec.insert("biomes", Tag::Compound(f));
    }
    changed
}

fn legal_bits(len: usize) -> u8 {
    let need = (usize::BITS - len.saturating_sub(1).leading_zeros()).max(1) as u8;
    [1u8, 2, 3, 4, 5, 6, 8, 16].into_iter().find(|&b| b >= need).unwrap_or(16)
}

/// Paint one slot (blocks of the box inside it); returns blocks changed.
pub fn paint_slot(slot: &mut BedrockBiomeSlot, below: Option<&BedrockBiomeSlot>, sel: Sel, cx: i32, cz: i32, base_y: i32, id: i32) -> u64 {
    if matches!(slot, BedrockBiomeSlot::NonExistent) {
        *slot = below.cloned().filter(|s| !matches!(s, BedrockBiomeSlot::NonExistent)).unwrap_or(BedrockBiomeSlot::Storage { palette: vec![id], indices: None, bits: 0 });
    }
    let BedrockBiomeSlot::Storage { palette, indices, bits } = slot else { return 0 };
    let mut idx = indices.clone().unwrap_or_else(|| vec![0; 4096]);
    let pid = match palette.iter().position(|&p| p == id) {
        Some(i) => i as u16,
        None => {
            palette.push(id);
            (palette.len() - 1) as u16
        }
    };
    let (x0, x1) = (sel.min[0].max(cx * 16), sel.max[0].min(cx * 16 + 15));
    let (z0, z1) = (sel.min[2].max(cz * 16), sel.max[2].min(cz * 16 + 15));
    let (y0, y1) = (sel.min[1].max(base_y), sel.max[1].min(base_y + 15));
    let mut n = 0;
    for x in x0..=x1 {
        for z in z0..=z1 {
            for y in y0..=y1 {
                let i = block_index_xzy((x - cx * 16) as usize, (y - base_y) as usize, (z - cz * 16) as usize);
                if idx[i] != pid {
                    idx[i] = pid;
                    n += 1;
                }
            }
        }
    }
    // Compact.
    let mut used: Vec<u16> = idx.clone();
    used.sort_unstable();
    used.dedup();
    let new_pal: Vec<i32> = used.iter().map(|&u| palette[u as usize]).collect();
    if new_pal.len() == 1 {
        *indices = None;
        *bits = 0;
    } else {
        let remap: std::collections::HashMap<u16, u16> = used.iter().enumerate().map(|(i, &u)| (u, i as u16)).collect();
        *indices = Some(idx.iter().map(|v| remap[v]).collect());
        *bits = legal_bits(new_pal.len());
    }
    *palette = new_pal;
    n
}

fn paint_bedrock(w: &BedrockWorld, dim: u8, sel: Sel, id: i32, min_y: i32, cancel: &(dyn Fn() -> bool + Sync)) -> Result<(Report, Undo)> {
    let mut report = Report::default();
    let mut undo = Vec::new();
    for (cx, cz) in sel.chunks() {
        if cancel() {
            break;
        }
        let key = keys::data3d_key(cx, cz, dim);
        let Some(raw) = w.raw_get(&key)? else {
            report.missing_chunks += 1;
            continue;
        };
        let mut d = Data3d::parse(&raw).map_err(|e| EditError::Corrupt(e.to_string()))?;
        let mut n = 0;
        for s in (sel.min[1] - min_y).div_euclid(16)..=(sel.max[1] - min_y).div_euclid(16) {
            let s = s as usize;
            while d.subchunks.len() <= s {
                d.subchunks.push(BedrockBiomeSlot::NonExistent);
            }
            let below = if s > 0 { d.subchunks[..s].iter().rev().find(|x| !matches!(x, BedrockBiomeSlot::NonExistent)).cloned() } else { None };
            n += paint_slot(&mut d.subchunks[s], below.as_ref(), sel, cx, cz, min_y + s as i32 * 16, id);
        }
        if n > 0 {
            w.raw_put_many(&[(key.clone(), d.serialize())])?;
            undo.push((key, Some(raw)));
            report.changed += n;
            report.chunks += 1;
        }
    }
    Ok((report, Undo::Bedrock(undo)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_cells_paint_and_compact() {
        let sec = Tag::Compound(vec![("Y".into(), Tag::Byte(4)), ("biomes".into(), Tag::Compound(vec![("palette".into(), Tag::List(vec![Tag::String("minecraft:plains".into())]))]))]);
        let mut chunk = Tag::Compound(vec![("sections".into(), Tag::List(vec![sec]))]);
        // Cells x 0..=1 (blocks 0..7), z 0, y 64..67 → 2 cells.
        let n = paint_java_chunk(&mut chunk, 0, 0, Sel::new([0, 64, 0], [7, 67, 3]), "minecraft:desert");
        assert_eq!(n, 2);
        let b = chunk.get("sections").and_then(Tag::as_list).unwrap()[0].get("biomes").unwrap().clone();
        let pal = b.get("palette").and_then(Tag::as_list).unwrap();
        assert_eq!(pal.len(), 2);
        let data = match b.get("data") {
            Some(Tag::LongArray(d)) => unpack_java_biome_cells(d, 2),
            _ => panic!("data"),
        };
        let desert = pal.iter().position(|t| t.as_str() == Some("minecraft:desert")).unwrap() as u16;
        assert_eq!(data[0], desert);
        assert_eq!(data[1], desert);
        assert_ne!(data[2], desert);
        // Painting everything compacts back to one entry.
        paint_java_chunk(&mut chunk, 0, 0, Sel::new([0, 64, 0], [15, 79, 15]), "minecraft:desert");
        let b = chunk.get("sections").and_then(Tag::as_list).unwrap()[0].get("biomes").unwrap().clone();
        assert_eq!(b.get("palette").and_then(Tag::as_list).unwrap().len(), 1);
        assert!(b.get("data").is_none());
    }

    #[test]
    fn bedrock_slot_paint_materializes_and_compacts() {
        let below = BedrockBiomeSlot::Storage { palette: vec![1], indices: None, bits: 0 };
        let mut slot = BedrockBiomeSlot::NonExistent;
        let n = paint_slot(&mut slot, Some(&below), Sel::new([0, 0, 0], [0, 0, 0]), 0, 0, 0, 2);
        assert_eq!(n, 1);
        let BedrockBiomeSlot::Storage { palette, indices, bits } = &slot else { panic!() };
        assert_eq!(palette, &vec![1, 2]);
        assert_eq!(*bits, 1);
        assert_eq!(indices.as_ref().unwrap()[0], 1);
        assert!(bedrock_biome_id("plains").is_some());
        assert!(bedrock_biome_id("not_a_biome").is_none());
        // Round trip through the data3D codec.
        let d = Data3d { heightmap: vec![0; 256], subchunks: vec![slot] };
        assert_eq!(Data3d::parse(&d.serialize()).unwrap(), d);
    }
}
