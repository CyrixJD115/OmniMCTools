//! The pieces of the Lua host other crates build on (`omt-extensions`):
//! the shared globals (`TYPE`, `EXTENSION_TYPE`, `FILE_TYPE`, `EDITION`,
//! `print` / `logger` into a log, extension-relative `require`) and the tag
//! userdata — so tools, NBT actions and NBT-editor style runs all speak the
//! same tag API (`child`, `getInt`, `setValue`, `addCompound`, …).

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use mlua::{AnyUserData, Lua};
use omt_nbt::Tag;

use crate::host::{Arena, LuaTag};
use crate::{Context, Edition};

/// Shared log sink of a Lua state: `(level, message)`.
pub type Logs = Rc<RefCell<Vec<(String, String)>>>;

/// Install the standard globals into `lua`; `base` is the extensions folder
/// (`require("<id>.module")` resolves there).
pub fn install_globals(lua: &Lua, base: &Path, logs: Logs) -> mlua::Result<()> {
    crate::host::setup_globals(lua, base, &Context { file_type: crate::file_type::GENERIC, edition: Edition::Unknown }, logs)
}

/// A tag handle over a fresh copy of `tag` (named `name`), with the full
/// tag API. Edits stay in the handle until read back with [`tag_of`].
pub fn new_tag(lua: &Lua, name: &str, tag: &Tag) -> mlua::Result<AnyUserData> {
    let arena = Rc::new(RefCell::new(Arena::build(name, tag)));
    lua.create_userdata(LuaTag { idx: 0, arena })
}

/// Whether a userdata is a tag handle.
pub fn is_tag(ud: &AnyUserData) -> bool {
    ud.is::<LuaTag>()
}

/// The current value of a tag handle (its subtree) and its name.
pub fn tag_of(ud: &AnyUserData) -> mlua::Result<(String, Tag)> {
    let t = ud.borrow::<LuaTag>()?;
    let a = t.arena.borrow();
    Ok((a.get(t.idx).name.clone(), a.materialize(t.idx)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_round_trip_through_lua_edits() {
        let lua = Lua::new();
        install_globals(&lua, Path::new("/nonexistent"), Logs::default()).unwrap();
        let root = Tag::Compound(vec![("Count".into(), Tag::Byte(3)), ("Name".into(), Tag::String("minecraft:stone".into()))]);
        let ud = new_tag(&lua, "item", &root).unwrap();
        lua.globals().set("item", ud.clone()).unwrap();
        lua.load(r#"item:setByte("Count", 7); item:addInt("Damage", 5)"#).exec().unwrap();
        let (name, tag) = tag_of(&ud).unwrap();
        assert_eq!(name, "item");
        let Tag::Compound(e) = tag else { panic!() };
        assert_eq!(e[0], ("Count".into(), Tag::Byte(7)));
        assert!(e.iter().any(|(k, v)| k == "Damage" && *v == Tag::Int(5)));
        assert!(is_tag(&ud));
        // EXTENSION_TYPE carries the platform types.
        assert_eq!(lua.load("return EXTENSION_TYPE.TOOL + EXTENSION_TYPE.NBT_ACTION").eval::<i64>().unwrap(), 5);
    }
}
