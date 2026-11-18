//! Structure files: Bedrock `.mcstructure` (uncompressed little-endian
//! NBT; `block_indices` nested x → y → z with z fastest, a second layer of
//! −1 for "no waterlog block", palette `default.block_palette`) and Java
//! structure `.nbt` (gzip big-endian; `palette` + `blocks[{pos,state}]`).
//! Import produces a [`Volume`] to paste with [`crate::Op::Paste`].

use omt_nbt::{ByteOrder, Container, Nbt, Tag};
use omt_world::BlockState;

use crate::{Block, EditError, Result, Volume, VolumeBuilder};

fn ints(v: &[i32]) -> Tag {
    Tag::List(v.iter().map(|&i| Tag::Int(i)).collect())
}

fn corrupt(e: impl std::fmt::Display) -> EditError {
    EditError::Corrupt(e.to_string())
}

/// Bedrock `.mcstructure` bytes. `version` = block data version for
/// palette entries without their own NBT.
pub fn to_mcstructure(vol: &Volume, origin: [i32; 3], version: Option<i32>) -> Result<Vec<u8>> {
    let [sx, sy, sz] = vol.size;
    let n = (sx * sy * sz) as usize;
    let mut primary = Vec::with_capacity(n);
    for x in 0..sx {
        for y in 0..sy {
            for z in 0..sz {
                primary.push(Tag::Int(vol.idx[((y * sz + z) * sx + x) as usize] as i32));
            }
        }
    }
    let palette: Vec<Tag> = vol.palette.iter().map(|b| crate::bedrock::raw_entry_pub(b, version)).collect();
    let structure = Tag::Compound(vec![
        ("block_indices".into(), Tag::List(vec![Tag::List(primary), Tag::List(vec![Tag::Int(-1); n])])),
        ("entities".into(), Tag::List(Vec::new())),
        (
            "palette".into(),
            Tag::Compound(vec![("default".into(), Tag::Compound(vec![("block_palette".into(), Tag::List(palette)), ("block_position_data".into(), Tag::Compound(Vec::new()))]))]),
        ),
    ]);
    let root = Tag::Compound(vec![
        ("format_version".into(), Tag::Int(1)),
        ("size".into(), ints(&vol.size)),
        ("structure".into(), structure),
        ("structure_world_origin".into(), ints(&origin)),
    ]);
    omt_nbt::write(&Nbt { name: String::new(), tag: root }, ByteOrder::Little).map_err(corrupt)
}

/// Java structure `.nbt` bytes (gzip).
pub fn to_java_structure(vol: &Volume, data_version: i32) -> Result<Vec<u8>> {
    let [sx, sy, sz] = vol.size;
    let palette: Vec<Tag> = vol
        .palette
        .iter()
        .map(|b| {
            let mut f = vec![("Name".to_string(), Tag::String(b.state.name.clone()))];
            if !b.state.properties.is_empty() {
                f.push(("Properties".into(), Tag::Compound(b.state.properties.iter().map(|(k, v)| (k.clone(), Tag::String(v.clone()))).collect())));
            }
            Tag::Compound(f)
        })
        .collect();
    let mut blocks = Vec::with_capacity(vol.idx.len());
    for y in 0..sy {
        for z in 0..sz {
            for x in 0..sx {
                let s = vol.idx[((y * sz + z) * sx + x) as usize];
                blocks.push(Tag::Compound(vec![("pos".into(), ints(&[x, y, z])), ("state".into(), Tag::Int(s as i32))]));
            }
        }
    }
    let root = Tag::Compound(vec![
        ("DataVersion".into(), Tag::Int(data_version)),
        ("size".into(), ints(&vol.size)),
        ("palette".into(), Tag::List(palette)),
        ("blocks".into(), Tag::List(blocks)),
        ("entities".into(), Tag::List(Vec::new())),
    ]);
    let raw = omt_nbt::write(&Nbt { name: String::new(), tag: root }, ByteOrder::Big).map_err(corrupt)?;
    Ok(Container::Gzip.encode(&raw))
}

fn size_of(t: &Tag) -> Option<[i32; 3]> {
    let l = t.get("size")?.as_list()?;
    let g = |i: usize| l.get(i).and_then(Tag::as_i64).map(|v| v as i32);
    let s = [g(0)?, g(1)?, g(2)?];
    (s.iter().all(|&v| (1..=4096).contains(&v)) && (s[0] as i64 * s[1] as i64 * s[2] as i64) <= 64 << 20).then_some(s)
}

fn props_of(t: Option<&Tag>) -> Vec<(String, String)> {
    t.and_then(Tag::as_compound)
        .map(|c| {
            c.iter()
                .map(|(k, v)| {
                    let s = match v {
                        Tag::String(s) => s.clone(),
                        other => omt_nbt::snbt::to_snbt(other),
                    };
                    (k.clone(), s)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Read either structure format (detected by content).
pub fn import(bytes: &[u8]) -> Result<Volume> {
    let (raw, container) = Container::decode(bytes).map_err(corrupt)?;
    if container == Container::Raw
        && let Ok(n) = omt_nbt::read(&raw, ByteOrder::Little)
        && n.tag.get("structure").is_some()
    {
        return from_mcstructure(&n.tag);
    }
    let n = omt_nbt::read(&raw, ByteOrder::Big).map_err(corrupt)?;
    from_java_structure(&n.tag)
}

fn from_mcstructure(t: &Tag) -> Result<Volume> {
    let size = size_of(t).ok_or_else(|| EditError::Corrupt("bad structure size".into()))?;
    let st = t.get("structure").ok_or_else(|| EditError::Corrupt("no structure".into()))?;
    let pal = st.get("palette").and_then(|p| p.get("default")).and_then(|d| d.get("block_palette")).and_then(Tag::as_list).ok_or_else(|| EditError::Corrupt("no palette".into()))?;
    let layers = st.get("block_indices").and_then(Tag::as_list).ok_or_else(|| EditError::Corrupt("no block_indices".into()))?;
    let primary = layers.first().and_then(Tag::as_list).ok_or_else(|| EditError::Corrupt("empty block_indices".into()))?;
    let mut vb = VolumeBuilder::new(size);
    let ids: Vec<u32> = pal
        .iter()
        .map(|e| {
            let name = e.get("name").and_then(Tag::as_str).unwrap_or("minecraft:air").to_string();
            vb.id(&Block { state: BlockState { name, properties: props_of(e.get("states")) }, bedrock_raw: Some(e.clone()) })
        })
        .collect();
    let [sx, sy, sz] = size;
    let mut i = 0;
    for x in 0..sx {
        for y in 0..sy {
            for z in 0..sz {
                if let Some(v) = primary.get(i).and_then(Tag::as_i64)
                    && v >= 0
                    && let Some(&id) = ids.get(v as usize)
                {
                    vb.set(x, y, z, id);
                }
                i += 1;
            }
        }
    }
    Ok(vb.finish())
}

fn from_java_structure(t: &Tag) -> Result<Volume> {
    let size = size_of(t).ok_or_else(|| EditError::Corrupt("bad structure size".into()))?;
    let pal = t.get("palette").and_then(Tag::as_list).ok_or_else(|| EditError::Corrupt("no palette".into()))?;
    let mut vb = VolumeBuilder::new(size);
    let ids: Vec<u32> = pal
        .iter()
        .map(|e| {
            let name = e.get("Name").and_then(Tag::as_str).unwrap_or("minecraft:air").to_string();
            vb.id(&Block { state: BlockState { name, properties: props_of(e.get("Properties")) }, bedrock_raw: None })
        })
        .collect();
    for b in t.get("blocks").and_then(Tag::as_list).into_iter().flatten() {
        let Some(p) = b.get("pos").and_then(Tag::as_list) else { continue };
        let g = |i: usize| p.get(i).and_then(Tag::as_i64).map(|v| v as i32);
        let (Some(x), Some(y), Some(z)) = (g(0), g(1), g(2)) else { continue };
        let Some(&id) = b.get("state").and_then(Tag::as_i64).and_then(|s| ids.get(s as usize)) else { continue };
        if x >= 0 && y >= 0 && z >= 0 && x < size[0] && y < size[1] && z < size[2] {
            vb.set(x, y, z, id);
        }
    }
    Ok(vb.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Volume {
        let mut vb = VolumeBuilder::new([3, 2, 4]);
        let stone = vb.id(&Block::named("stone"));
        let log = vb.id(&Block { state: BlockState { name: "minecraft:oak_log".into(), properties: vec![("axis".into(), "y".into())] }, bedrock_raw: None });
        vb.set(0, 0, 0, stone);
        vb.set(2, 1, 3, log);
        vb.finish()
    }

    #[test]
    fn mcstructure_roundtrip_and_layout() {
        let v = sample();
        let bytes = to_mcstructure(&v, [10, 64, -5], Some(18_100_737)).unwrap();
        let n = omt_nbt::read(&bytes, ByteOrder::Little).unwrap();
        let idx = n.tag.get("structure").unwrap().get("block_indices").unwrap().as_list().unwrap()[0].as_list().unwrap().to_vec();
        // (x=2, y=1, z=3) is the last entry with z fastest.
        assert_eq!(idx.last(), Some(&Tag::Int(2)));
        let back = import(&bytes).unwrap();
        assert_eq!(back.size, [3, 2, 4]);
        assert_eq!(back.at(2, 1, 3).unwrap().state.name, "minecraft:oak_log");
        assert_eq!(back.at(0, 0, 0).unwrap().state.name, "minecraft:stone");
        assert_eq!(back.at(1, 0, 0).unwrap().state.name, "minecraft:air");
    }

    #[test]
    fn java_structure_roundtrip() {
        let v = sample();
        let bytes = to_java_structure(&v, 4671).unwrap();
        assert_eq!(&bytes[..2], &[0x1f, 0x8b], "gzip");
        let back = import(&bytes).unwrap();
        assert_eq!(back.at(2, 1, 3).unwrap().state, BlockState { name: "minecraft:oak_log".into(), properties: vec![("axis".into(), "y".into())] });
        assert!(import(b"garbage").is_err());
    }
}
