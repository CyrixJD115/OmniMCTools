//! Host-neutral execution of [`omt_plugin_abi::HostRequest`]: the world /
//! NBT / item / storage / effect operations the `omt.*` API offers, run
//! without any Lua state. Native plugins call these through the loader's
//! host callback; the result is a [`omt_plugin_abi::HostResponse`].
//!
//! NBT values cross as SNBT. The same operations back the Lua API too (that
//! path stays in [`crate::world_api`] for the userdata ergonomics); this
//! module is the plain-data twin used across the plugin boundary.

use std::path::PathBuf;
use std::sync::Arc;

use omt_nbt::{NbtDocument, NbtFormat, Tag};
use omt_plugin_abi::{Container, HostRequest, HostResponse, Item, NamedTag, Rect, WorldInfo};
use omt_world::{Edition, WorldNode};

use crate::runtime::{Effects, Shared, WorldEntry, WorldHandle, WorldOpener};

/// Everything a run of host calls shares with the app.
pub struct HostCtx {
    pub world: Option<WorldHandle>,
    pub opener: Option<WorldOpener>,
    pub library: Vec<WorldEntry>,
    pub ext_dir: PathBuf,
    pub shared: Arc<Shared>,
    pub effects: Effects,
}

fn err(m: impl Into<String>) -> HostResponse {
    HostResponse::Err { message: m.into() }
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

fn be_pos(t: &Tag) -> Option<(i32, i32, i32)> {
    Some((int(get(t, "x"))?, int(get(t, "y"))?, int(get(t, "z"))?))
}

fn be_id(t: &Tag) -> String {
    match get(t, "id") {
        Some(Tag::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn roman(n: i32) -> String {
    ["", "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"].get(n as usize).map_or_else(|| n.to_string(), |s| s.to_string())
}

/// Normalize an [`crate::items::Item`] into the wire shape the GUI renders
/// (label, composed lore, glint), matching the Lua chest items.
pub fn wire_item(it: &crate::items::Item) -> Item {
    let mut lore: Vec<String> = it.enchantments.iter().map(|(id, l)| format!("{} {}", crate::api::pretty_id(id), roman(*l))).collect();
    lore.extend(it.lore.iter().cloned());
    if it.damage > 0 {
        lore.push(format!("Damage: {}", it.damage));
    }
    if !it.contents.is_empty() {
        lore.push(format!("{} stack(s) inside", it.contents.len()));
    }
    Item {
        slot: it.slot,
        id: it.id.clone(),
        count: it.count,
        damage: it.damage,
        label: it.custom_name.clone(),
        lore,
        enchantments: it.enchantments.clone(),
        glint: !it.enchantments.is_empty(),
        contents: it.contents.iter().map(wire_item).collect(),
    }
}

fn snbt(t: &Tag) -> String {
    omt_nbt::snbt::to_snbt(t)
}

fn parse(s: &str) -> Result<Tag, HostResponse> {
    omt_nbt::snbt::from_snbt(s).map_err(|e| err(format!("SNBT: {e}")))
}

fn read_doc(bytes: &[u8], name: &str) -> Option<Tag> {
    NbtDocument::from_bytes(std::path::Path::new(name), bytes).ok().map(|d| d.root().clone())
}

fn rect_chunks(h: &WorldHandle, r: Rect, shared: &Shared) -> Result<Vec<omt_world::Chunk>, HostResponse> {
    let (cx0, cz0, cx1, cz1) = (r.x0.min(r.x1).div_euclid(16), r.z0.min(r.z1).div_euclid(16), r.x0.max(r.x1).div_euclid(16), r.z0.max(r.z1).div_euclid(16));
    let area = i64::from(cx1 - cx0 + 1) * i64::from(cz1 - cz0 + 1);
    let coords: Vec<(i32, i32)> = if area > 4096 {
        h.store.iter_chunks(r.dim).map_err(|e| err(e.to_string()))?.filter(|&(x, z)| x >= cx0 && x <= cx1 && z >= cz0 && z <= cz1).collect()
    } else {
        (cz0..=cz1).flat_map(|z| (cx0..=cx1).map(move |x| (x, z))).collect()
    };
    let mut out = Vec::new();
    for (i, &(x, z)) in coords.iter().enumerate() {
        if shared.cancelled() {
            return Err(err("cancelled"));
        }
        if i % 16 == 0 {
            shared.set_progress(Some((format!("scanning chunk {} / {}", i + 1, coords.len()), i as f32 / coords.len().max(1) as f32)));
        }
        if let Some(c) = h.store.read_chunk(r.dim, x, z).map_err(|e| err(e.to_string()))? {
            out.push(c);
        }
    }
    shared.set_progress(None);
    Ok(out)
}

fn in_rect((x, _, z): (i32, i32, i32), r: Rect) -> bool {
    x >= r.x0.min(r.x1) && x <= r.x0.max(r.x1) && z >= r.z0.min(r.z1) && z <= r.z0.max(r.z1)
}

fn world(ctx: &HostCtx) -> Result<&WorldHandle, HostResponse> {
    ctx.world.as_ref().ok_or_else(|| err("no world is open"))
}

fn writer(h: &WorldHandle) -> Result<&Arc<dyn crate::runtime::WorldWriter>, HostResponse> {
    h.writer.as_ref().filter(|_| h.writable).ok_or_else(|| err("this world is open read-only — open it in the NBT Editor or World Editor to edit"))
}

/// A world as the plugin sees it in `Ctx`.
pub fn world_info(h: &WorldHandle) -> WorldInfo {
    WorldInfo { name: h.name.clone(), edition: if h.edition == Edition::Java { "JAVA".into() } else { "BEDROCK".into() }, path: h.path.display().to_string(), writable: h.writable }
}

fn storage_file(ctx: &HostCtx) -> PathBuf {
    ctx.ext_dir.join("storage.json")
}

fn storage_load(p: &std::path::Path) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

const MAX_RESULTS: usize = 20_000;

/// Run one host request against `ctx`.
pub fn execute(req: &HostRequest, ctx: &mut HostCtx) -> HostResponse {
    match req {
        HostRequest::Cancelled => HostResponse::Bool { value: ctx.shared.cancelled() },
        HostRequest::ReplaceNode { snbt: s } => match parse(s) {
            Ok(t) => {
                ctx.effects.replace = Some(t);
                HostResponse::Unit
            }
            Err(e) => e,
        },
        HostRequest::Notify { message } => {
            ctx.effects.notes.push(message.clone());
            HostResponse::Unit
        }
        HostRequest::Clipboard { text } => {
            ctx.effects.clipboard = Some(text.clone());
            HostResponse::Unit
        }
        HostRequest::Progress { text, fraction } => {
            ctx.shared.set_progress(text.clone().map(|t| (t, *fraction)));
            HostResponse::Unit
        }
        HostRequest::Log { level, message } => {
            ctx.effects.logs.push((level.clone(), message.clone()));
            HostResponse::Unit
        }
        HostRequest::StorageGet { key } => match storage_load(&storage_file(ctx)).remove(key) {
            Some(v) => HostResponse::Json { value: v },
            None => HostResponse::Json { value: serde_json::Value::Null },
        },
        HostRequest::StorageSet { key, value } => {
            let p = storage_file(ctx);
            let mut m = storage_load(&p);
            match value {
                Some(v) => {
                    m.insert(key.clone(), v.clone());
                }
                None => {
                    m.remove(key);
                }
            }
            let bytes = serde_json::to_vec_pretty(&m).unwrap_or_default();
            let tmp = p.with_extension("json.tmp");
            match std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &p)) {
                Ok(()) => HostResponse::Unit,
                Err(e) => err(e.to_string()),
            }
        }
        HostRequest::WorldList => HostResponse::Worlds {
            value: ctx.library.iter().map(|w| WorldInfo { name: w.name.clone(), edition: if w.edition == Edition::Java { "JAVA".into() } else { "BEDROCK".into() }, path: w.path.display().to_string(), writable: false }).collect(),
        },
        HostRequest::Containers { rect } => containers(ctx, *rect),
        HostRequest::BlockEntities { rect, filter } => block_entities(ctx, *rect, filter.as_deref()),
        HostRequest::Block { dim, x, y, z } => block(ctx, *dim, *x, *y, *z),
        HostRequest::Players => players(ctx),
        HostRequest::Level => match world(ctx) {
            Ok(h) => HostResponse::Text { value: h.store.read_raw(&WorldNode::LevelDat).ok().and_then(|b| read_doc(&b, "level.dat")).as_ref().map(snbt) },
            Err(e) => e,
        },
        HostRequest::Chunks { dim } => match world(ctx) {
            Ok(h) => match h.store.iter_chunks(*dim) {
                Ok(it) => HostResponse::Chunks { value: it.take(4_000_000).collect() },
                Err(e) => err(e.to_string()),
            },
            Err(e) => e,
        },
        HostRequest::SetBlockEntity { dim, x, y, z, snbt: s } => set_block_entity(ctx, *dim, *x, *y, *z, s),
        HostRequest::SetBlock { dim, x, y, z, block } => match world(ctx).and_then(|h| writer(h).map(|w| (h, w))) {
            Ok((_, w)) => match w.fill(*dim, [*x, *y, *z], [*x, *y, *z], block, None) {
                Ok(n) => {
                    ctx.effects.dirty = true;
                    HostResponse::Bool { value: n > 0 }
                }
                Err(e) => err(e),
            },
            Err(e) => e,
        },
        HostRequest::Fill { dim, a, b, block, replace } => fill(ctx, *dim, *a, *b, block, replace.as_deref()),
        HostRequest::ItemsRead { snbt: s } => match parse(s) {
            Ok(mut t) => {
                if let Tag::Compound(e) = &t {
                    for key in ["Items", "Inventory", "EnderItems"] {
                        if let Some((_, l)) = e.iter().find(|(k, _)| k == key) {
                            t = l.clone();
                            break;
                        }
                    }
                }
                HostResponse::Items { value: crate::items::items(&t).iter().map(wire_item).collect() }
            }
            Err(e) => e,
        },
        HostRequest::ItemMake { id, count, slot, java } => HostResponse::Text { value: Some(snbt(&crate::items::make(id, *count, *slot, *java))) },
        HostRequest::NbtRead { path } => match NbtDocument::load(std::path::Path::new(path)) {
            Ok(d) => HostResponse::Text { value: Some(snbt(d.root())) },
            Err(e) => err(e.to_string()),
        },
        HostRequest::NbtWrite { path, snbt: s, format } => nbt_write(path, s, format),
        HostRequest::NbtParse { snbt: s } => match parse(s) {
            Ok(_) => HostResponse::Bool { value: true },
            Err(e) => e,
        },
        HostRequest::NbtSnbt { snbt: s, pretty } => match parse(s) {
            Ok(t) => HostResponse::Text { value: Some(if *pretty { omt_nbt::snbt::to_snbt_pretty(&t) } else { snbt(&t) }) },
            Err(e) => e,
        },
    }
}

fn containers(ctx: &mut HostCtx, rect: Rect) -> HostResponse {
    let h = match world(ctx) {
        Ok(h) => h.clone(),
        Err(e) => return e,
    };
    let chunks = match rect_chunks(&h, rect, &ctx.shared) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let mut out = Vec::new();
    for c in chunks {
        for b in &c.block_entities {
            let Some(p) = be_pos(b) else { continue };
            let kind = crate::items::container_kind(&be_id(b));
            let Some((rows, cols)) = crate::items::layout(&kind) else { continue };
            if !in_rect(p, rect) || out.len() >= MAX_RESULTS {
                continue;
            }
            let items = get(b, "Items").map(crate::items::items).unwrap_or_default().iter().map(wire_item).collect();
            out.push(Container { x: p.0, y: p.1, z: p.2, kind, rows, cols, items, snbt: snbt(b) });
        }
    }
    HostResponse::Containers { value: out }
}

fn block_entities(ctx: &mut HostCtx, rect: Rect, filter: Option<&str>) -> HostResponse {
    let h = match world(ctx) {
        Ok(h) => h.clone(),
        Err(e) => return e,
    };
    let chunks = match rect_chunks(&h, rect, &ctx.shared) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let mut out = Vec::new();
    for c in chunks {
        for b in &c.block_entities {
            let Some(p) = be_pos(b) else { continue };
            if !in_rect(p, rect) || out.len() >= MAX_RESULTS {
                continue;
            }
            let id = be_id(b);
            let kind = crate::items::container_kind(&id);
            if filter.is_none_or(|f| kind.contains(f)) {
                out.push(omt_plugin_abi::BlockEntity { x: p.0, y: p.1, z: p.2, id, kind, snbt: snbt(b) });
            }
        }
    }
    HostResponse::BlockEntities { value: out }
}

fn block(ctx: &HostCtx, dim: u8, x: i32, y: i32, z: i32) -> HostResponse {
    let h = match world(ctx) {
        Ok(h) => h,
        Err(e) => return e,
    };
    let c = match h.store.read_chunk(dim, x.div_euclid(16), z.div_euclid(16)) {
        Ok(Some(c)) => c,
        Ok(None) => return HostResponse::Block { name: None, properties: Vec::new() },
        Err(e) => return err(e.to_string()),
    };
    match c.block_at(x.rem_euclid(16) as usize, y, z.rem_euclid(16) as usize) {
        Some(b) => HostResponse::Block { name: Some(b.name.clone()), properties: b.properties.clone() },
        None => HostResponse::Block { name: None, properties: Vec::new() },
    }
}

fn players(ctx: &HostCtx) -> HostResponse {
    let h = match world(ctx) {
        Ok(h) => h,
        Err(e) => return e,
    };
    let mut ids: Vec<String> = Vec::new();
    if h.edition == Edition::Java {
        for d in ["playerdata", "players"] {
            for e in std::fs::read_dir(h.path.join(d)).into_iter().flatten().flatten() {
                if let Some(id) = e.file_name().to_string_lossy().strip_suffix(".dat") {
                    ids.push(id.to_string());
                }
            }
        }
        ids.sort();
        ids.dedup();
    } else {
        ids.push("local".into());
    }
    let value = ids.into_iter().filter_map(|id| h.store.read_raw(&WorldNode::PlayerFile(id.clone())).ok().and_then(|b| read_doc(&b, &id)).map(|t| NamedTag { id, snbt: snbt(&t) })).collect();
    HostResponse::Tags { value }
}

fn set_block_entity(ctx: &mut HostCtx, dim: u8, x: i32, y: i32, z: i32, s: &str) -> HostResponse {
    let h = match world(ctx) {
        Ok(h) => h.clone(),
        Err(e) => return e,
    };
    let w = match writer(&h) {
        Ok(w) => w.clone(),
        Err(e) => return e,
    };
    let mut t = match parse(s) {
        Ok(t) => t,
        Err(e) => return e,
    };
    if let Tag::Compound(e) = &mut t {
        for (k, v) in [("x", x), ("y", y), ("z", z)] {
            match e.iter_mut().find(|(n, _)| n == k) {
                Some((_, slot)) => *slot = Tag::Int(v),
                None => e.push((k.to_string(), Tag::Int(v))),
            }
        }
    } else {
        return err("set_block_entity: tag must be a compound");
    }
    let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
    let mut c = match h.store.read_chunk(dim, cx, cz) {
        Ok(Some(c)) => c,
        Ok(None) => return err(format!("no chunk at {cx}, {cz}")),
        Err(e) => return err(e.to_string()),
    };
    match c.block_entities.iter_mut().find(|b| be_pos(b) == Some((x, y, z))) {
        Some(slot) => *slot = t,
        None => c.block_entities.push(t),
    }
    match w.set_block_entities(dim, cx, cz, &c.block_entities) {
        Ok(()) => {
            ctx.effects.dirty = true;
            HostResponse::Bool { value: true }
        }
        Err(e) => err(e),
    }
}

fn fill(ctx: &mut HostCtx, dim: u8, a: [i32; 3], b: [i32; 3], block: &str, replace: Option<&str>) -> HostResponse {
    let vol = (u64::from(a[0].abs_diff(b[0])) + 1) * (u64::from(a[1].abs_diff(b[1])) + 1) * (u64::from(a[2].abs_diff(b[2])) + 1);
    if vol > 4_000_000 {
        return err(format!("fill: {vol} blocks is over the 4 000 000 limit — split the box"));
    }
    let h = match world(ctx) {
        Ok(h) => h.clone(),
        Err(e) => return e,
    };
    match writer(&h).map(Arc::clone) {
        Ok(w) => match w.fill(dim, a, b, block, replace) {
            Ok(n) => {
                ctx.effects.dirty = true;
                HostResponse::Count { value: n }
            }
            Err(e) => err(e),
        },
        Err(e) => e,
    }
}

fn nbt_write(path: &str, s: &str, format: &str) -> HostResponse {
    let format = match format {
        "java" => NbtFormat::JavaGzip,
        "java_raw" => NbtFormat::RawBigEndian,
        "bedrock" => NbtFormat::RawLittleEndian,
        "bedrock_level" => NbtFormat::BedrockLevelDat,
        other => return err(format!("unknown NBT format {other:?}")),
    };
    let t = match omt_nbt::snbt::from_snbt(s) {
        Ok(t) => t,
        Err(e) => return err(format!("SNBT: {e}")),
    };
    let doc = NbtDocument { path: PathBuf::from(path), format, nbt: omt_nbt::Nbt { name: String::new(), tag: t }, bedrock_storage_version: 10, list_hints: Default::default() };
    match doc.save() {
        Ok(_) => HostResponse::Bool { value: true },
        Err(e) => err(e.to_string()),
    }
}
