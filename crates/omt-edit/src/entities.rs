//! Entity editing inside a selection: list (counts by type) and remove
//! (all, or one type), with undo.
//! * Bedrock: each chunk's `digp` record lists 8-byte actor ids; actors
//!   live under `actorprefix` + id. Removing rewrites the `digp` list and
//!   deletes the actor records.
//! * Java: `entities/r.x.z.mca` chunks hold an `Entities` list; removing
//!   filters it (region rewritten, other chunks verbatim).

use std::collections::BTreeMap;

use omt_bedrock::BedrockWorld;
use omt_bedrock::keys;
use omt_nbt::Tag;

use crate::{EditWorld, Report, Result, Sel, Undo};

/// One entity found in the selection.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityInfo {
    pub id: String,
    pub pos: [f64; 3],
}

fn pos_of(t: &Tag) -> Option<[f64; 3]> {
    let l = t.get("Pos")?.as_list()?;
    let f = |i: usize| match l.get(i)? {
        Tag::Double(v) => Some(*v),
        Tag::Float(v) => Some(*v as f64),
        other => other.as_i64().map(|v| v as f64),
    };
    Some([f(0)?, f(1)?, f(2)?])
}

fn inside(sel: &Sel, p: [f64; 3]) -> bool {
    sel.contains(p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32)
}

fn matches(id: &str, filter: Option<&str>) -> bool {
    filter.is_none_or(|f| {
        let f = f.trim();
        f.is_empty() || id == f || id.trim_start_matches("minecraft:") == f.trim_start_matches("minecraft:")
    })
}

/// Entities inside `sel`.
pub fn list(w: &EditWorld, dim: u8, sel: Sel) -> Result<Vec<EntityInfo>> {
    let mut out = Vec::new();
    match w {
        EditWorld::Bedrock(b) => {
            for (cx, cz) in sel.chunks() {
                for (_, tag) in bedrock_actors(b, dim, cx, cz)? {
                    if let (Some(id), Some(p)) = (tag.get("identifier").and_then(Tag::as_str), pos_of(&tag))
                        && inside(&sel, p)
                    {
                        out.push(EntityInfo { id: id.to_string(), pos: p });
                    }
                }
            }
        }
        EditWorld::Java(j) => {
            let Some(dir) = j.entity_dimension_dir(dim) else { return Ok(out) };
            for (cx, cz) in sel.chunks() {
                let Ok(region) = omt_java::RegionFile::open(&dir.join(format!("r.{}.{}.mca", cx.div_euclid(32), cz.div_euclid(32)))) else { continue };
                if !region.contains(cx, cz) {
                    continue;
                }
                let Ok(n) = region.read_chunk_from_file(cx, cz) else { continue };
                for e in n.tag.get("Entities").and_then(Tag::as_list).into_iter().flatten() {
                    if let (Some(id), Some(p)) = (e.get("id").and_then(Tag::as_str), pos_of(e))
                        && inside(&sel, p)
                    {
                        out.push(EntityInfo { id: id.to_string(), pos: p });
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Counts by type, most common first.
pub fn counts(list: &[EntityInfo]) -> Vec<(String, u64)> {
    let mut m: BTreeMap<&str, u64> = BTreeMap::new();
    for e in list {
        *m.entry(e.id.as_str()).or_default() += 1;
    }
    let mut v: Vec<(String, u64)> = m.into_iter().map(|(k, n)| (k.to_string(), n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// (actor id, NBT) for a Bedrock chunk.
fn bedrock_actors(w: &BedrockWorld, dim: u8, cx: i32, cz: i32) -> Result<Vec<([u8; 8], Tag)>> {
    let Some(digp) = w.raw_get(&keys::digp_key(cx, cz, dim))? else { return Ok(Vec::new()) };
    let ids: Vec<[u8; 8]> = digp.as_chunks::<8>().0.to_vec();
    let vals = w.get_exact(&ids.iter().map(|id| keys::actor_key(*id)).collect::<Vec<_>>())?;
    Ok(ids.into_iter().zip(vals).filter_map(|(id, v)| Some((id, omt_bedrock::store::parse_actor_nbt(&v?).ok()?.tag))).collect())
}

/// Remove entities inside `sel` (optionally only `filter` type).
/// Players are never removed.
pub fn remove(w: &EditWorld, dim: u8, sel: Sel, filter: Option<&str>, cancel: &(dyn Fn() -> bool + Sync)) -> Result<(Report, Undo)> {
    let keep_player = |id: &str| id.trim_start_matches("minecraft:") == "player";
    match w {
        EditWorld::Bedrock(b) => {
            let mut report = Report::default();
            let mut undo: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
            for (cx, cz) in sel.chunks() {
                if cancel() {
                    break;
                }
                let digp_key = keys::digp_key(cx, cz, dim);
                let Some(digp) = b.raw_get(&digp_key)? else { continue };
                let mut keep: Vec<u8> = Vec::with_capacity(digp.len());
                let mut gone: Vec<Vec<u8>> = Vec::new();
                for (id, tag) in bedrock_actors(b, dim, cx, cz)? {
                    let eid = tag.get("identifier").and_then(Tag::as_str).unwrap_or("");
                    let hit = pos_of(&tag).is_some_and(|p| inside(&sel, p)) && matches(eid, filter) && !keep_player(eid);
                    if hit {
                        let k = keys::actor_key(id);
                        undo.push((k.clone(), b.raw_get(&k)?));
                        gone.push(k);
                    } else {
                        keep.extend_from_slice(&id);
                    }
                }
                // Ids whose actor record was missing stay listed as found.
                let listed: usize = digp.len() / 8;
                if gone.is_empty() || listed == 0 {
                    continue;
                }
                undo.push((digp_key.clone(), Some(digp.clone())));
                b.raw_delete_many(&gone)?;
                b.raw_put_many(&[(digp_key, keep)])?;
                report.changed += gone.len() as u64;
                report.chunks += 1;
            }
            Ok((report, Undo::Bedrock(undo)))
        }
        EditWorld::Java(j) => crate::java::for_chunks(j, dim, sel, true, cancel, &mut |t, _, _| {
            let Some(Tag::List(list)) = t.get_mut("Entities") else { return Ok((0, 0)) };
            let before = list.len();
            list.retain(|e| {
                let id = e.get("id").and_then(Tag::as_str).unwrap_or("");
                !(pos_of(e).is_some_and(|p| inside(&sel, p)) && matches(id, filter) && !keep_player(id))
            });
            Ok(((before - list.len()) as u64, 0))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_and_counts() {
        assert!(matches("minecraft:zombie", Some("zombie")));
        assert!(matches("minecraft:zombie", Some("minecraft:zombie")));
        assert!(matches("minecraft:zombie", None));
        assert!(!matches("minecraft:cow", Some("zombie")));
        let e = |id: &str| EntityInfo { id: id.into(), pos: [0.0; 3] };
        let c = counts(&[e("minecraft:cow"), e("minecraft:zombie"), e("minecraft:cow")]);
        assert_eq!(c[0], ("minecraft:cow".to_string(), 2));
        let t = Tag::Compound(vec![("Pos".into(), Tag::List(vec![Tag::Float(1.5), Tag::Float(64.0), Tag::Float(-3.2)]))]);
        assert_eq!(pos_of(&t), Some([1.5, 64.0, -3.200000047683716]));
        assert!(inside(&Sel::new([0, 60, -5], [5, 70, 0]), [1.5, 64.0, -3.2]));
    }
}
