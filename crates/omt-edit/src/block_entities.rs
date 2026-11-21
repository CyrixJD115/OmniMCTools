//! Replace a chunk's block entities (container contents, signs, spawners…)
//! in place, both editions: Bedrock rewrites the chunk's `0x31` record (a
//! sequence of little-endian NBT roots), Java rewrites the chunk inside its
//! region file (`block_entities`, or `Level.TileEntities` before 1.18) —
//! atomically, with undo data like every other edit.

use omt_bedrock::keys::{self, tag};
use omt_nbt::{ByteOrder, Nbt, Tag};

use crate::{EditError, EditWorld, Result, Sel, Undo};

/// Replace the block entities of chunk (`cx`, `cz`) with `list` (the
/// complete new set). Errors when the chunk does not exist.
pub fn set_block_entities(w: &EditWorld, dim: u8, cx: i32, cz: i32, list: &[Tag]) -> Result<Undo> {
    match w {
        EditWorld::Bedrock(b) => {
            let key = keys::chunk_record(cx, cz, dim, tag::BLOCK_ENTITIES);
            // The chunk must exist (its version record is always present).
            if b.raw_get(&keys::chunk_record(cx, cz, dim, tag::VERSION))?.is_none() && b.raw_get(&keys::chunk_record(cx, cz, dim, tag::VERSION_OLD))?.is_none() {
                return Err(EditError::Other(format!("no chunk at {cx}, {cz}")));
            }
            let old = b.raw_get(&key)?;
            if list.is_empty() {
                if old.is_some() {
                    b.raw_delete_many(std::slice::from_ref(&key))?;
                }
            } else {
                let mut out = Vec::new();
                for t in list {
                    out.extend(omt_nbt::write(&Nbt { name: String::new(), tag: t.clone() }, ByteOrder::Little).map_err(|e| EditError::Corrupt(e.to_string()))?);
                }
                b.raw_put_many(&[(key.clone(), out)])?;
            }
            Ok(Undo::Bedrock(vec![(key, old)]))
        }
        EditWorld::Java(j) => {
            let sel = Sel::new([cx * 16, 0, cz * 16], [cx * 16 + 15, 0, cz * 16 + 15]);
            let mut found = false;
            let (_, undo) = crate::java::for_chunks(j, dim, sel, false, &|| false, &mut |chunk, x, z| {
                if (x, z) != (cx, cz) {
                    return Ok((0, 0));
                }
                found = true;
                let new = Tag::List(list.to_vec());
                // 1.18+ keeps them at the root; older chunks under `Level`.
                if chunk.get("block_entities").is_some() || chunk.get("Level").is_none() {
                    chunk.insert("block_entities", new);
                } else if let Some(level) = chunk.get_mut("Level") {
                    level.insert("TileEntities", new);
                }
                Ok((1, 0))
            })?;
            if !found {
                return Err(EditError::Other(format!("no chunk at {cx}, {cz}")));
            }
            Ok(undo)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Java: a fixture region with one chunk; replace its block entities
    /// and read them back.
    #[test]
    fn java_block_entities_round_trip() {
        let dir = std::env::temp_dir().join(format!("omt-edit-be-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // A minimal Java world: level.dat + one region holding chunk (2, -1).
        std::fs::create_dir_all(dir.join("region")).unwrap();
        let level = Nbt { name: String::new(), tag: Tag::Compound(vec![("Data".into(), Tag::Compound(vec![("DataVersion".into(), Tag::Int(3953))]))]) };
        std::fs::write(dir.join("level.dat"), omt_nbt::Container::Raw.encode(&omt_nbt::write(&level, ByteOrder::Big).unwrap())).unwrap();
        let chunk = Nbt { name: String::new(), tag: Tag::Compound(vec![("xPos".into(), Tag::Int(2)), ("zPos".into(), Tag::Int(-1)), ("sections".into(), Tag::List(Vec::new())), ("block_entities".into(), Tag::List(Vec::new()))]) };
        let path = dir.join("region/r.0.-1.mca");
        let empty = vec![0u8; omt_java::region::HEADER_BYTES];
        let region = omt_java::RegionFile::from_bytes(&path, &empty).unwrap();
        std::fs::write(&path, region.with_chunk(&empty, 2, -1, &omt_nbt::write(&chunk, ByteOrder::Big).unwrap(), 0)).unwrap();
        let w = std::sync::Arc::new(omt_java::JavaWorld::open(&dir).unwrap());
        let chest = Tag::Compound(vec![("id".into(), Tag::String("minecraft:chest".into())), ("x".into(), Tag::Int(33)), ("y".into(), Tag::Int(64)), ("z".into(), Tag::Int(-10))]);
        let ew = EditWorld::Java(w.clone());
        set_block_entities(&ew, 0, 2, -1, std::slice::from_ref(&chest)).unwrap();
        let nbt = w.read_chunk_nbt(0, 2, -1).unwrap().unwrap();
        assert_eq!(nbt.tag.get("block_entities"), Some(&Tag::List(vec![chest])));
        assert!(set_block_entities(&ew, 0, 9, 9, &[]).is_err(), "missing chunk");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
