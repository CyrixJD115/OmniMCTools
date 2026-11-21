//! The world object extensions get (`omt.world.current()`,
//! `omt.world.open(path)`): level.dat, players, chunks, blocks, block
//! entities, containers, entities — and block-entity writes on editable
//! sessions. Coordinates are block coordinates unless named `cx`/`cz`.

use std::cell::RefCell;

use mlua::{AnyUserData, Lua, Table, UserData, UserDataFields, UserDataMethods, Value};
use omt_nbt::Tag;
use omt_world::{Chunk, Edition, WorldNode};

use crate::api::item_table;
use crate::items;
use crate::runtime::{Rt, WorldHandle};

/// Most block entities / entities one query returns.
const MAX_RESULTS: usize = 20_000;

/// The last chunk read: `((dim, cx, cz), chunk)`.
type ChunkCache = Option<((u8, i32, i32), Option<Chunk>)>;

pub struct LuaWorld {
    pub h: WorldHandle,
    /// Last chunk read (block lookups walk neighbouring blocks).
    cache: RefCell<ChunkCache>,
}

impl LuaWorld {
    pub fn new(h: WorldHandle) -> Self {
        Self { h, cache: RefCell::new(None) }
    }

    fn chunk(&self, dim: u8, cx: i32, cz: i32) -> mlua::Result<Option<Chunk>> {
        if let Some((k, c)) = self.cache.borrow().as_ref()
            && *k == (dim, cx, cz)
        {
            return Ok(c.clone());
        }
        let c = self.h.store.read_chunk(dim, cx, cz).map_err(mlua::Error::external)?;
        *self.cache.borrow_mut() = Some(((dim, cx, cz), c.clone()));
        Ok(c)
    }

    /// Present chunks overlapping a block rectangle (progress + cancel).
    fn chunks_in(&self, lua: &Lua, dim: u8, (x0, z0, x1, z1): (i32, i32, i32, i32), what: &str) -> mlua::Result<Vec<Chunk>> {
        let (cx0, cz0, cx1, cz1) = (x0.min(x1).div_euclid(16), z0.min(z1).div_euclid(16), x0.max(x1).div_euclid(16), z0.max(z1).div_euclid(16));
        let area = (cx1 - cx0 + 1) as i64 * (cz1 - cz0 + 1) as i64;
        // Huge rectangles: only visit chunks that exist.
        let coords: Vec<(i32, i32)> = if area > 4096 {
            let it = self.h.store.iter_chunks(dim).map_err(mlua::Error::external)?;
            it.filter(|&(x, z)| x >= cx0 && x <= cx1 && z >= cz0 && z <= cz1).collect()
        } else {
            (cz0..=cz1).flat_map(|z| (cx0..=cx1).map(move |x| (x, z))).collect()
        };
        let rt = lua.app_data_ref::<Rt>().ok_or_else(|| mlua::Error::runtime("no runtime"))?;
        let mut out = Vec::new();
        for (i, &(x, z)) in coords.iter().enumerate() {
            rt.check()?;
            if i % 16 == 0 {
                rt.shared.set_progress(Some((format!("{what}: chunk {} / {}", i + 1, coords.len()), i as f32 / coords.len().max(1) as f32)));
            }
            if let Some(c) = self.h.store.read_chunk(dim, x, z).map_err(mlua::Error::external)? {
                out.push(c);
            }
        }
        rt.shared.set_progress(None);
        Ok(out)
    }
}

fn get<'a>(t: &'a Tag, key: &str) -> Option<&'a Tag> {
    match t {
        Tag::Compound(e) => e.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

fn int(t: Option<&Tag>) -> Option<i32> {
    match t? {
        Tag::Byte(v) => Some(i32::from(*v)),
        Tag::Short(v) => Some(i32::from(*v)),
        Tag::Int(v) => Some(*v),
        _ => None,
    }
}

/// `x, y, z` of a block entity.
fn be_pos(t: &Tag) -> Option<(i32, i32, i32)> {
    Some((int(get(t, "x"))?, int(get(t, "y"))?, int(get(t, "z"))?))
}

fn be_id(t: &Tag) -> String {
    match get(t, "id") {
        Some(Tag::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn in_rect((x, _, z): (i32, i32, i32), (x0, z0, x1, z1): (i32, i32, i32, i32)) -> bool {
    x >= x0.min(x1) && x <= x0.max(x1) && z >= z0.min(z1) && z <= z0.max(z1)
}

fn tag_ud(lua: &Lua, name: &str, t: &Tag) -> mlua::Result<AnyUserData> {
    omt_script::lua::new_tag(lua, name, t)
}

/// A block entity as the table extensions get.
fn be_table(lua: &Lua, t: &Tag) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    if let Some((x, y, z)) = be_pos(t) {
        out.set("x", x)?;
        out.set("y", y)?;
        out.set("z", z)?;
    }
    let id = be_id(t);
    out.set("kind", items::container_kind(&id))?;
    out.set("id", id)?;
    out.set("tag", tag_ud(lua, "", t)?)?;
    Ok(out)
}

fn read_doc(bytes: &[u8], name: &str) -> Option<Tag> {
    omt_nbt::NbtDocument::from_bytes(std::path::Path::new(name), bytes).ok().map(|d| d.root().clone())
}

impl UserData for LuaWorld {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("name", |_, w| Ok(w.h.name.clone()));
        f.add_field_method_get("path", |_, w| Ok(w.h.path.display().to_string()));
        f.add_field_method_get("edition", |_, w| Ok(if w.h.edition == Edition::Java { "JAVA" } else { "BEDROCK" }));
        f.add_field_method_get("writable", |_, w| Ok(w.h.writable));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // level.dat as a tag (nil when unreadable).
        m.add_method("level", |lua, w, ()| match w.h.store.read_raw(&WorldNode::LevelDat).ok().and_then(|b| read_doc(&b, "level.dat")) {
            Some(t) => Ok(Value::UserData(tag_ud(lua, "", &t)?)),
            None => Ok(Value::Nil),
        });
        // One player: Java uuid, Bedrock "local" (or a raw player key).
        m.add_method("player", |lua, w, name: String| match w.h.store.read_raw(&WorldNode::PlayerFile(name.clone())).ok().and_then(|b| read_doc(&b, &name)) {
            Some(t) => Ok(Value::UserData(tag_ud(lua, &name, &t)?)),
            None => Ok(Value::Nil),
        });
        // Every player found: { {id=…, tag=…}, … }.
        m.add_method("players", |lua, w, ()| {
            let mut ids: Vec<String> = Vec::new();
            if w.h.edition == Edition::Java {
                // `playerdata/<uuid>.dat`; worlds before 1.7.6 used `players/<name>.dat`.
                for e in ["playerdata", "players"].iter().flat_map(|d| std::fs::read_dir(w.h.path.join(d)).into_iter().flatten().flatten()) {
                    let n = e.file_name().to_string_lossy().into_owned();
                    if let Some(id) = n.strip_suffix(".dat") {
                        ids.push(id.to_string());
                    }
                }
                ids.sort();
            } else {
                ids.push("local".into());
            }
            let out = lua.create_table()?;
            for id in ids {
                if let Some(t) = w.h.store.read_raw(&WorldNode::PlayerFile(id.clone())).ok().and_then(|b| read_doc(&b, &id)) {
                    let p = lua.create_table()?;
                    p.set("id", id.as_str())?;
                    p.set("tag", tag_ud(lua, &id, &t)?)?;
                    out.push(p)?;
                }
            }
            Ok(out)
        });
        // Present chunk coordinates: { {x=, z=}, … } (chunk coordinates).
        m.add_method("chunks", |lua, w, dim: Option<u8>| {
            let it = w.h.store.iter_chunks(dim.unwrap_or(0)).map_err(mlua::Error::external)?;
            let out = lua.create_table()?;
            for (x, z) in it.take(4_000_000) {
                let c = lua.create_table()?;
                c.set("x", x)?;
                c.set("z", z)?;
                out.push(c)?;
            }
            Ok(out)
        });
        // One chunk (chunk coordinates): block entities, entities, metadata.
        m.add_method("chunk", |lua, w, (dim, cx, cz): (u8, i32, i32)| {
            let Some(c) = w.chunk(dim, cx, cz)? else { return Ok(Value::Nil) };
            let t = lua.create_table()?;
            t.set("x", c.x)?;
            t.set("z", c.z)?;
            t.set("data_version", c.data_version)?;
            t.set("inhabited_time", c.inhabited_time)?;
            t.set("status", c.status.clone())?;
            let bes = lua.create_table()?;
            for b in &c.block_entities {
                bes.push(be_table(lua, b)?)?;
            }
            t.set("block_entities", bes)?;
            let ents = lua.create_table()?;
            for e in &c.entities {
                ents.push(tag_ud(lua, "", e)?)?;
            }
            t.set("entities", ents)?;
            Ok(Value::Table(t))
        });
        // Block at (x, y, z): name, { property = value }.
        m.add_method("block", |lua, w, (dim, x, y, z): (u8, i32, i32, i32)| {
            let Some(c) = w.chunk(dim, x.div_euclid(16), z.div_euclid(16))? else { return Ok((Value::Nil, Value::Nil)) };
            let Some(b) = c.block_at(x.rem_euclid(16) as usize, y, z.rem_euclid(16) as usize) else { return Ok((Value::Nil, Value::Nil)) };
            let props = lua.create_table()?;
            for (k, v) in &b.properties {
                props.set(k.as_str(), v.as_str())?;
            }
            Ok((Value::String(lua.create_string(&b.name)?), Value::Table(props)))
        });
        // Block entities in a block rectangle; `filter` = kind substring
        // (`"chest"`, `"sign"`) or function(be) → bool.
        m.add_method("block_entities", |lua, w, (dim, x0, z0, x1, z1, filter): (u8, i32, i32, i32, i32, Value)| {
            let rect = (x0, z0, x1, z1);
            let out = lua.create_table()?;
            let mut n = 0usize;
            for c in w.chunks_in(lua, dim, rect, "block entities")? {
                for b in &c.block_entities {
                    let Some(p) = be_pos(b) else { continue };
                    if !in_rect(p, rect) || n >= MAX_RESULTS {
                        continue;
                    }
                    let kind = items::container_kind(&be_id(b));
                    let keep = match &filter {
                        Value::String(s) => kind.contains(&*s.to_str()?),
                        Value::Function(f) => f.call::<bool>(be_table(lua, b)?)?,
                        _ => true,
                    };
                    if keep {
                        out.push(be_table(lua, b)?)?;
                        n += 1;
                    }
                }
            }
            Ok(out)
        });
        // Containers (chests, barrels, shulkers, hoppers…) with their items
        // and grid layout: { {x,y,z,kind,rows,cols,items={…},tag}, … }.
        m.add_method("containers", |lua, w, (dim, x0, z0, x1, z1): (u8, i32, i32, i32, i32)| {
            let rect = (x0, z0, x1, z1);
            let out = lua.create_table()?;
            for c in w.chunks_in(lua, dim, rect, "containers")? {
                for b in &c.block_entities {
                    let Some(p) = be_pos(b) else { continue };
                    let kind = items::container_kind(&be_id(b));
                    let Some((rows, cols)) = items::layout(&kind) else { continue };
                    if !in_rect(p, rect) || out.raw_len() >= MAX_RESULTS {
                        continue;
                    }
                    let t = be_table(lua, b)?;
                    t.set("rows", rows)?;
                    t.set("cols", cols)?;
                    let list = lua.create_table()?;
                    for it in get(b, "Items").map(items::items).unwrap_or_default() {
                        list.push(item_table(lua, &it)?)?;
                    }
                    t.set("items", list)?;
                    out.push(t)?;
                }
            }
            Ok(out)
        });
        // Entities in a block rectangle: { {id, x, y, z, tag}, … }.
        m.add_method("entities", |lua, w, (dim, x0, z0, x1, z1): (u8, i32, i32, i32, i32)| {
            let out = lua.create_table()?;
            for c in w.chunks_in(lua, dim, (x0, z0, x1, z1), "entities")? {
                for e in &c.entities {
                    let pos: Vec<f64> = match get(e, "Pos") {
                        Some(Tag::List(l)) => l.iter().filter_map(|v| match v {
                            Tag::Double(d) => Some(*d),
                            Tag::Float(f) => Some(f64::from(*f)),
                            _ => None,
                        }).collect(),
                        _ => Vec::new(),
                    };
                    let [x, y, z] = pos[..] else { continue };
                    if !in_rect((x.floor() as i32, 0, z.floor() as i32), (x0, z0, x1, z1)) || out.raw_len() >= MAX_RESULTS {
                        continue;
                    }
                    let t = lua.create_table()?;
                    let id = match get(e, "id").or_else(|| get(e, "identifier")) {
                        Some(Tag::String(s)) => s.clone(),
                        _ => String::new(),
                    };
                    t.set("id", id)?;
                    t.set("x", x)?;
                    t.set("y", y)?;
                    t.set("z", z)?;
                    t.set("tag", tag_ud(lua, "", e)?)?;
                    out.push(t)?;
                }
            }
            Ok(out)
        });
        // Replace (or add) the block entity at (x, y, z) with `tag`
        // (editable sessions only; save the session to keep it).
        m.add_method("set_block_entity", |lua, w, (dim, x, y, z, tag): (u8, i32, i32, i32, AnyUserData)| {
            let writer = writer_of(&w.h)?;
            let (_, mut t) = omt_script::lua::tag_of(&tag)?;
            if let Tag::Compound(e) = &mut t {
                for (k, v) in [("x", x), ("y", y), ("z", z)] {
                    match e.iter_mut().find(|(n, _)| n == k) {
                        Some((_, slot)) => *slot = Tag::Int(v),
                        None => e.push((k.to_string(), Tag::Int(v))),
                    }
                }
            } else {
                return Err(mlua::Error::runtime("set_block_entity: tag must be a compound"));
            }
            let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
            let mut c = w.h.store.read_chunk(dim, cx, cz).map_err(mlua::Error::external)?.ok_or_else(|| mlua::Error::runtime("no chunk there"))?;
            match c.block_entities.iter_mut().find(|b| be_pos(b) == Some((x, y, z))) {
                Some(slot) => *slot = t,
                None => c.block_entities.push(t),
            }
            writer.set_block_entities(dim, cx, cz, &c.block_entities).map_err(mlua::Error::runtime)?;
            wrote(lua, w);
            Ok(true)
        });
        // One block: `w:set_block(dim, x, y, z, "oak_log[axis=y]")`. Block
        // entities at changed positions are removed (as in the game).
        m.add_method("set_block", |lua, w, (dim, x, y, z, block): (u8, i32, i32, i32, String)| {
            let n = writer_of(&w.h)?.fill(dim, [x, y, z], [x, y, z], &block, None).map_err(mlua::Error::runtime)?;
            wrote(lua, w);
            Ok(n > 0)
        });
        // A box: `w:fill(dim, x0, y0, z0, x1, y1, z1, block, { replace = "dirt" })`
        // → blocks changed (at most 4 million per call).
        m.add_method("fill", |lua, w, (dim, x0, y0, z0, x1, y1, z1, block, opts): (u8, i32, i32, i32, i32, i32, i32, String, Option<Table>)| {
            let vol = (u64::from((x1 - x0).unsigned_abs()) + 1) * (u64::from((y1 - y0).unsigned_abs()) + 1) * (u64::from((z1 - z0).unsigned_abs()) + 1);
            if vol > 4_000_000 {
                return Err(mlua::Error::runtime(format!("fill: {vol} blocks is over the 4 000 000 limit — split the box")));
            }
            let replace: Option<String> = opts.map(|o| o.get("replace")).transpose()?.flatten();
            if let Some(rt) = lua.app_data_ref::<Rt>() {
                rt.check()?;
            }
            let n = writer_of(&w.h)?.fill(dim, [x0, y0, z0], [x1, y1, z1], &block, replace.as_deref()).map_err(mlua::Error::runtime)?;
            wrote(lua, w);
            Ok(n)
        });
    }
}

fn writer_of(h: &WorldHandle) -> mlua::Result<std::sync::Arc<dyn crate::runtime::WorldWriter>> {
    h.writer.clone().filter(|_| h.writable).ok_or_else(|| mlua::Error::runtime("this world is open read-only (Map Viewer) — open it in the NBT Editor or World Editor to edit"))
}

/// After a write: drop the chunk cache, mark the session modified.
fn wrote(lua: &Lua, w: &LuaWorld) {
    *w.cache.borrow_mut() = None;
    if let Some(mut rt) = lua.app_data_mut::<Rt>() {
        rt.effects.dirty = true;
    }
}
