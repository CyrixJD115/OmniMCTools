//! The `omt` library every tool / NBT action gets:
//!
//! | | |
//! |---|---|
//! | `omt.world.current()` / `open(path)` / `list()` | worlds ([`crate::world_api`]) |
//! | `omt.nbt.read(path)` / `write(path, tag, format?)` / `parse(snbt)` / `snbt(tag, pretty?)` / `new(table)` | NBT files and values |
//! | `omt.items.read(listTag)` / `make{…}` / `layout(kind)` / `kind(id)` | item stacks, both editions |
//! | `omt.json.encode(v)` / `decode(s)` | JSON |
//! | `omt.storage.get(k)` / `set(k, v)` / `all()` | per-extension saved data |
//! | `omt.log(…)` / `notify(msg)` / `progress(text, f)` / `clipboard(text)` / `cancelled()` | app integration |

use std::path::{Path, PathBuf};

use mlua::{AnyUserData, Lua, LuaSerdeExt, Table, Value};
use omt_nbt::{NbtDocument, NbtFormat, Tag};

use crate::items::{self, Item};
use crate::runtime::Rt;
use crate::world_api::LuaWorld;

/// An item as a Lua table.
pub fn item_table(lua: &Lua, it: &Item) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("slot", it.slot)?;
    t.set("id", it.id.as_str())?;
    t.set("count", it.count)?;
    t.set("damage", it.damage)?;
    t.set("label", it.custom_name.clone())?;
    let mut lore: Vec<String> = it.enchantments.iter().map(|(id, l)| format!("{} {}", pretty_id(id), roman(*l))).collect();
    lore.extend(it.lore.iter().cloned());
    if it.damage > 0 {
        lore.push(format!("Damage: {}", it.damage));
    }
    if !it.contents.is_empty() {
        lore.push(format!("{} stack(s) inside", it.contents.len()));
    }
    t.set("lore", lore)?;
    let ench = lua.create_table()?;
    for (id, lvl) in &it.enchantments {
        let e = lua.create_table()?;
        e.set("id", id.as_str())?;
        e.set("level", *lvl)?;
        ench.push(e)?;
    }
    t.set("enchantments", ench)?;
    t.set("glint", !it.enchantments.is_empty())?;
    let contents = lua.create_table()?;
    for c in &it.contents {
        contents.push(item_table(lua, c)?)?;
    }
    t.set("contents", contents)?;
    Ok(t)
}

/// `minecraft:fire_aspect` → `Fire Aspect`.
pub fn pretty_id(id: &str) -> String {
    id.rsplit(':').next().unwrap_or(id).split('_').map(|w| {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    }).collect::<Vec<_>>().join(" ")
}

fn roman(n: i32) -> String {
    match n {
        1 => "I".into(),
        2 => "II".into(),
        3 => "III".into(),
        4 => "IV".into(),
        5 => "V".into(),
        6 => "VI".into(),
        7 => "VII".into(),
        8 => "VIII".into(),
        9 => "IX".into(),
        10 => "X".into(),
        n => n.to_string(),
    }
}

fn rt<'a>(lua: &'a Lua) -> mlua::Result<mlua::AppDataRefMut<'a, Rt>> {
    lua.app_data_mut::<Rt>().ok_or_else(|| mlua::Error::runtime("no extension runtime"))
}

fn tag_arg(v: &Value) -> mlua::Result<Tag> {
    match v {
        Value::UserData(ud) => Ok(omt_script::lua::tag_of(ud)?.1),
        _ => Err(mlua::Error::runtime("expected a tag")),
    }
}

/// Plain Lua value → NBT (integers → Int, floats → Double, strings,
/// booleans → Byte, sequences → List, other tables → Compound, tags as-is).
pub fn to_tag(v: &Value) -> mlua::Result<Tag> {
    Ok(match v {
        Value::Boolean(b) => Tag::Byte(i8::from(*b)),
        Value::Integer(i) => i32::try_from(*i).map_or(Tag::Long(*i), Tag::Int),
        Value::Number(n) => Tag::Double(*n),
        Value::String(s) => Tag::String(s.to_string_lossy()),
        Value::UserData(_) => tag_arg(v)?,
        Value::Table(t) => {
            let len = t.raw_len();
            if len > 0 {
                Tag::List(t.clone().sequence_values::<Value>().map(|x| to_tag(&x?)).collect::<mlua::Result<_>>()?)
            } else {
                let mut e: Vec<(String, Tag)> = t.clone().pairs::<String, Value>().map(|p| p.and_then(|(k, v)| Ok((k, to_tag(&v)?)))).collect::<mlua::Result<_>>()?;
                e.sort_by(|a, b| a.0.cmp(&b.0));
                Tag::Compound(e)
            }
        }
        other => return Err(mlua::Error::runtime(format!("cannot store {} in NBT", other.type_name()))),
    })
}

fn storage_path(lua: &Lua) -> mlua::Result<PathBuf> {
    Ok(rt(lua)?.ext_dir.join("storage.json"))
}

fn storage_load(p: &Path) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Install `omt` into `lua` (needs [`Rt`] as app data).
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let omt = lua.create_table()?;
    omt.set("version", env!("CARGO_PKG_VERSION"))?;

    // --- app integration ----------------------------------------------
    omt.set("log", lua.create_function(|lua, args: mlua::MultiValue| {
        let msg = args.iter().map(|v| v.to_string().unwrap_or_default()).collect::<Vec<_>>().join(" ");
        rt(lua)?.effects.logs.push(("info".into(), msg));
        Ok(())
    })?)?;
    omt.set("notify", lua.create_function(|lua, msg: String| {
        rt(lua)?.effects.notes.push(msg);
        Ok(())
    })?)?;
    omt.set("clipboard", lua.create_function(|lua, text: String| {
        rt(lua)?.effects.clipboard = Some(text);
        Ok(())
    })?)?;
    omt.set("progress", lua.create_function(|lua, (text, f): (Option<String>, Option<f32>)| {
        let r = rt(lua)?;
        r.check()?;
        r.shared.set_progress(text.map(|t| (t, f.unwrap_or(-1.0))));
        Ok(())
    })?)?;
    omt.set("cancelled", lua.create_function(|lua, ()| Ok(rt(lua)?.shared.cancelled()))?)?;

    // --- worlds ---------------------------------------------------------
    let world = lua.create_table()?;
    world.set("current", lua.create_function(|lua, ()| {
        let h = rt(lua)?.world.clone();
        h.map(|h| lua.create_userdata(LuaWorld::new(h))).transpose()
    })?)?;
    world.set("open", lua.create_function(|lua, path: String| {
        let opener = rt(lua)?.opener.clone().ok_or_else(|| mlua::Error::runtime("opening worlds is not available here"))?;
        let h = opener(Path::new(&path)).map_err(mlua::Error::runtime)?;
        lua.create_userdata(LuaWorld::new(h))
    })?)?;
    world.set("list", lua.create_function(|lua, ()| {
        let out = lua.create_table()?;
        for w in rt(lua)?.library.clone() {
            let t = lua.create_table()?;
            t.set("name", w.name)?;
            t.set("path", w.path.display().to_string())?;
            t.set("edition", if w.edition == omt_world::Edition::Java { "JAVA" } else { "BEDROCK" })?;
            out.push(t)?;
        }
        Ok(out)
    })?)?;
    omt.set("world", world)?;

    // --- NBT --------------------------------------------------------------
    let nbt = lua.create_table()?;
    nbt.set("read", lua.create_function(|lua, path: String| {
        let doc = NbtDocument::load(Path::new(&path)).map_err(mlua::Error::external)?;
        let name = doc.nbt.name.clone();
        let ud = omt_script::lua::new_tag(lua, &name, doc.root())?;
        Ok((ud, format!("{:?}", doc.format)))
    })?)?;
    // format: "java" (gzip, big-endian; default), "java_raw", "bedrock"
    // (raw little-endian), "bedrock_level" (level.dat header).
    nbt.set("write", lua.create_function(|_, (path, tag, format): (String, AnyUserData, Option<String>)| {
        let (name, t) = omt_script::lua::tag_of(&tag)?;
        let format = match format.as_deref().unwrap_or("java") {
            "java" => NbtFormat::JavaGzip,
            "java_raw" => NbtFormat::RawBigEndian,
            "bedrock" => NbtFormat::RawLittleEndian,
            "bedrock_level" => NbtFormat::BedrockLevelDat,
            other => return Err(mlua::Error::runtime(format!("unknown NBT format {other:?}"))),
        };
        let doc = NbtDocument { path: PathBuf::from(&path), format, nbt: omt_nbt::Nbt { name, tag: t }, bedrock_storage_version: 10, list_hints: Default::default() };
        doc.save().map_err(mlua::Error::external)?;
        Ok(true)
    })?)?;
    nbt.set("parse", lua.create_function(|lua, s: String| {
        let t = omt_nbt::snbt::from_snbt(&s).map_err(|e| mlua::Error::runtime(format!("SNBT: {e}")))?;
        omt_script::lua::new_tag(lua, "", &t)
    })?)?;
    nbt.set("snbt", lua.create_function(|_, (tag, pretty): (AnyUserData, Option<bool>)| {
        let (_, t) = omt_script::lua::tag_of(&tag)?;
        Ok(if pretty.unwrap_or(false) { omt_nbt::snbt::to_snbt_pretty(&t) } else { omt_nbt::snbt::to_snbt(&t) })
    })?)?;
    nbt.set("new", lua.create_function(|lua, v: Value| {
        let t = if v.is_nil() { Tag::Compound(Vec::new()) } else { to_tag(&v)? };
        omt_script::lua::new_tag(lua, "", &t)
    })?)?;
    // An empty list (or one from a Lua array of values / tags).
    nbt.set("list", lua.create_function(|lua, v: Option<Table>| {
        let items = match v {
            Some(t) => t.sequence_values::<Value>().map(|x| to_tag(&x?)).collect::<mlua::Result<Vec<_>>>()?,
            None => Vec::new(),
        };
        omt_script::lua::new_tag(lua, "", &Tag::List(items))
    })?)?;
    nbt.set("is_tag", lua.create_function(|_, v: Value| Ok(matches!(&v, Value::UserData(ud) if omt_script::lua::is_tag(ud))))?)?;
    omt.set("nbt", nbt)?;

    // --- items ---------------------------------------------------------
    let it = lua.create_table()?;
    // A list tag of items (or a container / player tag holding `Items` /
    // `Inventory`) → item tables.
    it.set("read", lua.create_function(|lua, v: Value| {
        let mut t = tag_arg(&v)?;
        if let Tag::Compound(e) = &t {
            for key in ["Items", "Inventory", "EnderItems"] {
                if let Some((_, l)) = e.iter().find(|(k, _)| k == key) {
                    t = l.clone();
                    break;
                }
            }
        }
        let out = lua.create_table()?;
        for item in items::items(&t) {
            out.push(item_table(lua, &item)?)?;
        }
        Ok(out)
    })?)?;
    // { id = "diamond", count = 3, slot = 0, edition = "JAVA"|"BEDROCK" }.
    it.set("make", lua.create_function(|lua, t: Table| {
        let id: String = t.get("id")?;
        let java = match t.get::<Option<String>>("edition")? {
            Some(e) => e.eq_ignore_ascii_case("java"),
            None => rt(lua)?.world.as_ref().is_none_or(|w| w.edition == omt_world::Edition::Java),
        };
        let tag = items::make(&id, t.get::<Option<i32>>("count")?.unwrap_or(1), t.get::<Option<i32>>("slot")?.unwrap_or(0), java);
        omt_script::lua::new_tag(lua, "", &tag)
    })?)?;
    it.set("layout", lua.create_function(|_, kind: String| Ok(items::layout(&items::container_kind(&kind)).map_or((None, None), |(r, c)| (Some(r), Some(c)))))?)?;
    it.set("kind", lua.create_function(|_, id: String| Ok(items::container_kind(&id)))?)?;
    it.set("name", lua.create_function(|_, id: String| Ok(pretty_id(&id)))?)?;
    omt.set("items", it)?;

    // --- JSON + storage ------------------------------------------------
    let json = lua.create_table()?;
    json.set("encode", lua.create_function(|lua, (v, pretty): (Value, Option<bool>)| {
        let j: serde_json::Value = lua.from_value(v)?;
        if pretty.unwrap_or(false) { serde_json::to_string_pretty(&j) } else { serde_json::to_string(&j) }.map_err(mlua::Error::external)
    })?)?;
    json.set("decode", lua.create_function(|lua, s: String| {
        let j: serde_json::Value = serde_json::from_str(&s).map_err(mlua::Error::external)?;
        lua.to_value(&j)
    })?)?;
    omt.set("json", json)?;

    let storage = lua.create_table()?;
    storage.set("get", lua.create_function(|lua, key: String| {
        let m = storage_load(&storage_path(lua)?);
        m.get(&key).map(|v| lua.to_value(v)).transpose()
    })?)?;
    storage.set("set", lua.create_function(|lua, (key, v): (String, Value)| {
        let p = storage_path(lua)?;
        let mut m = storage_load(&p);
        if v.is_nil() {
            m.remove(&key);
        } else {
            m.insert(key, lua.from_value(v)?);
        }
        let bytes = serde_json::to_vec_pretty(&m).map_err(mlua::Error::external)?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &p)).map_err(mlua::Error::external)?;
        Ok(())
    })?)?;
    storage.set("all", lua.create_function(|lua, ()| lua.to_value(&serde_json::Value::Object(storage_load(&storage_path(lua)?))))?)?;
    omt.set("storage", storage)?;

    lua.globals().set("omt", omt)?;
    // Lua-side helpers (written with the tag API itself).
    lua.load(
        r#"
        -- Put an item tag into a container / inventory tag: creates `Items`
        -- when the container has none (empty Java chests), replaces a stack
        -- already in that slot. Returns the container.
        function omt.items.put(container, item, key)
          key = key or "Items"
          local list = container:find(key)
          if not list then
            container:addList(key, omt.nbt.list())
            list = container:find(key)
          end
          local slot = item:getByte("Slot")
          for i = list.childCount - 1, 0, -1 do
            if slot ~= nil and list:child(i):getByte("Slot") == slot then list:removeChild(i) end
          end
          list:addChild(item)
          return container
        end

        -- Handy utilities (omt.util.*), written with the public API.
        omt.util = {}
        -- Total count of items whose id contains `needle` (""=all), including
        -- nested container contents.
        function omt.util.count(items, needle)
          local n = 0
          for _, it in ipairs(items) do
            if needle == nil or needle == "" or string.find(it.id, needle, 1, true) then n = n + it.count end
            for _, inner in ipairs(it.contents or {}) do
              if needle == nil or needle == "" or string.find(inner.id, needle, 1, true) then n = n + inner.count end
            end
          end
          return n
        end
        -- Clamp a number to [lo, hi].
        function omt.util.clamp(v, lo, hi) if v < lo then return lo elseif v > hi then return hi else return v end end
        -- Round to the nearest integer.
        function omt.util.round(v) return math.floor(v + 0.5) end
        -- "minecraft:fire_aspect" -> "Fire Aspect".
        function omt.util.title(id)
          local out = {}
          for word in string.gmatch(string.gsub(id, "^.*:", ""), "[^_]+") do
            out[#out + 1] = word:sub(1, 1):upper() .. word:sub(2)
          end
          return table.concat(out, " ")
        end
        -- Block coordinates -> chunk coordinates.
        function omt.util.chunk_of(x, z) return math.floor(x / 16), math.floor(z / 16) end
        "#,
    )
    .set_name("omt helpers")
    .exec()?;
    Ok(())
}
