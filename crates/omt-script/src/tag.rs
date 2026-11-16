//! The Lua tag userdata: properties, structural methods, typed accessors
//! and the OMT-defined styling calls. Semantics for methods UMT never
//! documented are OMT decisions — documented on each method (see
//! `EXTENSIONS.md` for the author-facing view).

use mlua::{AnyUserData, Lua, MetaMethod, MultiValue, Table, UserData, UserDataMethods, Value};
use omt_nbt::Tag;

use super::host::{lua_err, Arena, LuaTag, Node};
use crate::RowStyle;

/// Scalar/array tag → Lua value (containers → nil; use the child APIs).
fn value_of(lua: &Lua, tag: &Tag) -> mlua::Result<Value> {
    Ok(match tag {
        Tag::Byte(v) => Value::Integer(i64::from(*v)),
        Tag::Short(v) => Value::Integer(i64::from(*v)),
        Tag::Int(v) => Value::Integer(i64::from(*v)),
        Tag::Long(v) => Value::Integer(*v),
        Tag::Float(v) => Value::Number(f64::from(*v)),
        Tag::Double(v) => Value::Number(*v),
        Tag::String(s) => Value::String(lua.create_string(s.as_bytes())?),
        Tag::ByteArray(v) => int_table(lua, v.iter().map(|b| i64::from(*b)))?,
        Tag::IntArray(v) => int_table(lua, v.iter().map(|i| i64::from(*i)))?,
        Tag::LongArray(v) => int_table(lua, v.iter().copied())?,
        Tag::Compound(_) | Tag::List(_) => Value::Nil,
    })
}

fn int_table(lua: &Lua, vals: impl Iterator<Item = i64>) -> mlua::Result<Value> {
    let collected: Vec<i64> = vals.collect();
    let t = lua.create_table_with_capacity(collected.len(), 0)?;
    for (i, v) in collected.iter().enumerate() {
        t.raw_set(i + 1, *v)?;
    }
    Ok(Value::Table(t))
}

fn signed_list_from_table(t: &Table, min: i64, max: i64, what: &str) -> Result<Vec<i64>, mlua::Error> {
    let mut out = Vec::with_capacity(t.len().unwrap_or(0) as usize);
    for i in 1..=t.len()? {
        let v: Value = t.get(i)?;
        let Value::Integer(n) = v else {
            return Err(lua_err(format!("{what}: element {i} is not an integer")));
        };
        if !(min..=max).contains(&n) {
            return Err(lua_err(format!("{what}: element {i} ({n}) out of range {min}..={max}")));
        }
        out.push(n);
    }
    Ok(out)
}

/// Setter target: self (no selector) or an existing child (missing = error).
fn resolve(this: &LuaTag, sel: &Option<Value>) -> Result<u32, mlua::Error> {
    match sel {
        None => Ok(this.idx),
        Some(v) => { this.arena.borrow() }.child_by_sel(this.idx, v).ok_or_else(|| lua_err("no such child")),
    }
}

/// Getter target: like [`resolve`] but a missing child yields `Ok(None)`
/// (getters return nil for absent/wrongly-typed lookups — OMT contract).
fn resolve_opt(this: &LuaTag, sel: &Option<Value>) -> Result<Option<u32>, mlua::Error> {
    match sel {
        None => Ok(Some(this.idx)),
        Some(v) => Ok({ this.arena.borrow() }.child_by_sel(this.idx, v)),
    }
}

/// Setters are called as `t:setX(v)` (self) or `t:setX(sel, v)` (child);
/// split raw Lua args accordingly.
fn split_set_args(a: Value, b: Option<Value>) -> (Option<Value>, Value) {
    match b {
        Some(v) => (Some(a), v),
        None => (None, a),
    }
}

impl UserData for LuaTag {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // ---- properties (resolved after methods; see crate docs) ---------
        methods.add_meta_method(MetaMethod::Index, |lua, this, key: Value| {
            let key = match &key {
                Value::String(s) => s.to_string_lossy(),
                _ => return Ok(Value::Nil),
            };
            let a = this.arena.borrow();
            let node = a.get(this.idx);
            Ok(match key.as_str() {
                "name" => Value::String(lua.create_string(node.name.as_bytes())?),
                "value" => value_of(lua, &node.tag)?,
                "type" => Value::Integer(i64::from(node.tag.id())),
                "childCount" => Value::Integer(a.child_count(this.idx) as i64),
                "listType" => Value::Integer(i64::from(a.list_type(this.idx))),
                "parentTag" => match node.parent {
                    Some(p) => Value::UserData(lua.create_userdata(LuaTag { idx: p, arena: this.arena.clone() })?),
                    None => Value::Nil,
                },
                _ => Value::Nil,
            })
        });

        // ---- structure -----------------------------------------------------
        methods.add_method("hasParent", |_, this, ()| Ok(this.arena.borrow().get(this.idx).parent.is_some()));
        methods.add_method("getRow", |_, this, ()| Ok(this.arena.borrow().get(this.idx).row));
        methods.add_method("child", |_, this, i: i64| {
            let child = {
                let a = this.arena.borrow();
                if i < 0 || i as usize >= a.child_count(this.idx) {
                    None
                } else {
                    Some(a.get(this.idx).children[i as usize])
                }
            };
            Ok(child.map(|c| LuaTag { idx: c, arena: this.arena.clone() }))
        });
        methods.add_method("contains", |_, this, sel: Value| {
            Ok({ this.arena.borrow() }.child_by_sel(this.idx, &sel).is_some())
        });
        methods.add_method("find", |lua, this, name: String| {
            let sel = Value::String(lua.create_string(name.as_bytes())?);
            let found = { this.arena.borrow() }.child_by_sel(this.idx, &sel);
            Ok(found.map(|c| LuaTag { idx: c, arena: this.arena.clone() }))
        });
        methods.add_method("clone", |_, this, ()| {
            let mut a = this.arena.borrow_mut();
            let idx = a.copy_subtree(this.idx);
            Ok(LuaTag { idx, arena: this.arena.clone() })
        });
        methods.add_method("addChild", |_, this, other: AnyUserData| {
            let other = other.borrow::<LuaTag>()?;
            let mut a = this.arena.borrow_mut();
            let (kind, dup, list_type_now) = {
                let n = a.get(this.idx);
                let dup = matches!(&n.tag, Tag::Compound(_)) && n.children.iter().any(|&c| a.get(c).name == a.get(other.idx).name);
                let kind = match &n.tag {
                    Tag::Compound(_) => 1,
                    Tag::List(_) => 2,
                    _ => 0,
                };
                (kind, dup, a.list_type(this.idx))
            };
            if kind == 0 {
                return Err(lua_err("addChild: this tag is not a compound or list"));
            }
            if dup {
                return Err(lua_err("addChild: the compound already has a tag with that name"));
            }
            let src_id = a.get(other.idx).tag.id();
            if kind == 2 && list_type_now != 0 && list_type_now != src_id {
                return Err(lua_err("addChild: list elements must share one NBT type"));
            }
            a.attach_copy(this.idx, other.idx);
            Ok(())
        });
        methods.add_method("removeChild", |_, this, sel: Value| {
            let mut a = this.arena.borrow_mut();
            let child = a.child_by_sel(this.idx, &sel).ok_or_else(|| lua_err("removeChild: no such child"))?;
            a.get_mut(this.idx).children.retain(|&c| c != child);
            a.detach(child);
            a.renumber(this.idx);
            Ok(())
        });
        methods.add_method("clear", |_, this, ()| {
            let mut a = this.arena.borrow_mut();
            let children = a.get(this.idx).children.clone();
            for c in children {
                a.detach(c);
            }
            a.get_mut(this.idx).children.clear();
            Ok(())
        });
        methods.add_method("getSize", |_, this, ()| Ok({ this.arena.borrow() }.byte_size(this.idx) as i64));
        methods.add_method("getValue", |lua, this, ()| {
            let tag = { this.arena.borrow() }.get(this.idx).tag.clone();
            value_of(lua, &tag)
        });

        // ---- styling (OMT-defined) -----------------------------------------
        methods.add_method("setExtraText", |_, this, text: String| {
            this.arena.borrow_mut().nodes[this.idx as usize].style.extra_text = Some(text);
            Ok(())
        });
        methods.add_method("setDescription", |_, this, text: String| {
            this.arena.borrow_mut().nodes[this.idx as usize].style.description = Some(text);
            Ok(())
        });
        methods.add_method("setColor", |_, this, color: Value| {
            let rgb = parse_color(&color)?;
            this.arena.borrow_mut().nodes[this.idx as usize].style.color = Some(rgb);
            Ok(())
        });

        // ---- typed getters: (sel?) → value | nil ---------------------------
        methods.add_method("getByte", |_, this, sel: Option<Value>| {
            let Some(t) = resolve_opt(this, &sel)? else { return Ok(None); };
            Ok(match &this.arena.borrow().get(t).tag {
                Tag::Byte(v) => Some(i64::from(*v)),
                _ => None,
            })
        });
        methods.add_method("getBool", |_, this, sel: Option<Value>| {
            let Some(t) = resolve_opt(this, &sel)? else { return Ok(None); };
            Ok(match &this.arena.borrow().get(t).tag {
                Tag::Byte(v) => Some(*v != 0),
                _ => None,
            })
        });
        methods.add_method("getShort", |_, this, sel: Option<Value>| {
            let Some(t) = resolve_opt(this, &sel)? else { return Ok(None); };
            Ok(match &this.arena.borrow().get(t).tag {
                Tag::Short(v) => Some(i64::from(*v)),
                _ => None,
            })
        });
        methods.add_method("getInt", |_, this, sel: Option<Value>| {
            let Some(t) = resolve_opt(this, &sel)? else { return Ok(None); };
            Ok(match &this.arena.borrow().get(t).tag {
                Tag::Int(v) => Some(i64::from(*v)),
                _ => None,
            })
        });
        methods.add_method("getLong", |_, this, sel: Option<Value>| {
            let Some(t) = resolve_opt(this, &sel)? else { return Ok(None); };
            Ok(match &this.arena.borrow().get(t).tag {
                Tag::Long(v) => Some(*v),
                _ => None,
            })
        });
        methods.add_method("getFloat", |_, this, sel: Option<Value>| {
            let Some(t) = resolve_opt(this, &sel)? else { return Ok(None); };
            Ok(match &this.arena.borrow().get(t).tag {
                Tag::Float(v) => Some(f64::from(*v)),
                _ => None,
            })
        });
        methods.add_method("getDouble", |_, this, sel: Option<Value>| {
            let Some(t) = resolve_opt(this, &sel)? else { return Ok(None); };
            Ok(match &this.arena.borrow().get(t).tag {
                Tag::Double(v) => Some(*v),
                _ => None,
            })
        });
        methods.add_method("getString", |_, this, sel: Option<Value>| {
            let Some(t) = resolve_opt(this, &sel)? else { return Ok(None); };
            Ok(match &this.arena.borrow().get(t).tag {
                Tag::String(s) => Some(s.clone()),
                _ => None,
            })
        });
        methods.add_method("getByteArray", |lua, this, sel: Option<Value>| {
            let t = resolve(this, &sel)?;
            let tag = this.arena.borrow().get(t).tag.clone();
            Ok(match &tag {
                Tag::ByteArray(v) => Some(int_table(lua, v.iter().map(|b| i64::from(*b)))?),
                _ => None,
            })
        });
        methods.add_method("getIntArray", |lua, this, sel: Option<Value>| {
            let t = resolve(this, &sel)?;
            let tag = this.arena.borrow().get(t).tag.clone();
            Ok(match &tag {
                Tag::IntArray(v) => Some(int_table(lua, v.iter().map(|i| i64::from(*i)))?),
                _ => None,
            })
        });
        methods.add_method("getLongArray", |lua, this, sel: Option<Value>| {
            let t = resolve(this, &sel)?;
            let tag = this.arena.borrow().get(t).tag.clone();
            Ok(match &tag {
                Tag::LongArray(v) => Some(int_table(lua, v.iter().copied())?),
                _ => None,
            })
        });
        // Container getters return a tag handle; they never fail on type
        // (structure mismatches surface on first use).
        methods.add_method("getCompound", |_, this, sel: Option<Value>| {
            let t = resolve(this, &sel)?;
            Ok(LuaTag { idx: t, arena: this.arena.clone() })
        });
        methods.add_method("getList", |_, this, sel: Option<Value>| {
            let t = resolve(this, &sel)?;
            Ok(LuaTag { idx: t, arena: this.arena.clone() })
        });

        // ---- generic value set: type-preserving coercion --------------------
        methods.add_method("setValue", |_, this, (a, b): (Value, Option<Value>)| {
            let (sel, val) = split_set_args(a, b);
            let t = resolve(this, &sel)?;
            let new = coerce_preserving(&this.arena.borrow().get(t).tag.clone(), &val).map_err(lua_err)?;
            this.arena.borrow_mut().nodes[t as usize].tag = new;
            Ok(())
        });

        // ---- typed setters: (sel?, value) — the target must already be
        // that type (widths are part of NBT identity; widening is a rename). --
        macro_rules! set_int {
            ($name:literal, $variant:ident, $ty:ty, $min:expr, $max:expr) => {
                methods.add_method($name, |_, this, (a, b): (Value, Option<Value>)| {
                    let (sel, raw) = split_set_args(a, b);
                    let val: i64 = match raw {
                        Value::Integer(i) => i,
                        Value::Number(n) if n.fract() == 0.0 => n as i64,
                        other => return Err(lua_err(format!("{}: expected an integer, got {}", $name, other.type_name()))),
                    };
                    let t = resolve(this, &sel)?;
                    if !((i64::from($min))..=(i64::from($max))).contains(&val) {
                        return Err(lua_err(format!("{}: {val} out of range", $name)));
                    }
                    let mut a = this.arena.borrow_mut();
                    let node = a.get_mut(t);
                    if !matches!(node.tag, Tag::$variant(_)) {
                        return Err(lua_err(format!("{}: target is {} not {}", $name, node.tag.type_name(), stringify!($variant).to_lowercase())));
                    }
                    node.tag = Tag::$variant(val as $ty);
                    Ok(())
                });
            };
        }
        set_int!("setByte", Byte, i8, -128, 127);
        set_int!("setShort", Short, i16, -32768, 32767);
        set_int!("setInt", Int, i32, i32::MIN, i32::MAX);
        set_int!("setLong", Long, i64, i64::MIN, i64::MAX);

        macro_rules! set_float {
            ($name:literal, $variant:ident, $ty:ty) => {
                methods.add_method($name, |_, this, (a, b): (Value, Option<Value>)| {
                    let (sel, raw) = split_set_args(a, b);
                    let val: f64 = match raw {
                        Value::Integer(i) => i as f64,
                        Value::Number(n) => n,
                        other => return Err(lua_err(format!("{}: expected a number, got {}", $name, other.type_name()))),
                    };
                    let t = resolve(this, &sel)?;
                    let mut a = this.arena.borrow_mut();
                    let node = a.get_mut(t);
                    if !matches!(node.tag, Tag::$variant(_)) {
                        return Err(lua_err(format!("{}: target is {} not {}", $name, node.tag.type_name(), stringify!($variant).to_lowercase())));
                    }
                    node.tag = Tag::$variant(val as $ty);
                    Ok(())
                });
            };
        }
        set_float!("setFloat", Float, f32);
        set_float!("setDouble", Double, f64);

        methods.add_method("setBool", |_, this, (a, b): (Value, Option<Value>)| {
            let (sel, raw) = split_set_args(a, b);
            let val = matches!(raw, Value::Boolean(true)) || matches!(raw, Value::Integer(n) if n == 1);
            let t = resolve(this, &sel)?;
            let mut a = this.arena.borrow_mut();
            let node = a.get_mut(t);
            if !matches!(node.tag, Tag::Byte(_)) {
                return Err(lua_err(format!("setBool: target is {} not byte", node.tag.type_name())));
            }
            node.tag = Tag::Byte(i8::from(val));
            Ok(())
        });
        methods.add_method("setString", |_, this, (a, b): (Value, Option<Value>)| {
            let (sel, raw) = split_set_args(a, b);
            let Value::String(val) = raw else {
                return Err(lua_err("setString: expected a string"));
            };
            let val = val.to_string_lossy();
            let t = resolve(this, &sel)?;
            let mut a = this.arena.borrow_mut();
            let node = a.get_mut(t);
            if !matches!(node.tag, Tag::String(_)) {
                return Err(lua_err(format!("setString: target is {} not string", node.tag.type_name())));
            }
            node.tag = Tag::String(val);
            Ok(())
        });
        methods.add_method("setByteArray", |_, this, (a, b): (Value, Option<Value>)| {
            let (sel, raw) = split_set_args(a, b);
            let t = resolve(this, &sel)?;
            let val = table_arg(&raw, "setByteArray")?;
            let list = signed_list_from_table(&val, -128, 127, "setByteArray")?;
            let mut a = this.arena.borrow_mut();
            let node = a.get_mut(t);
            if !matches!(node.tag, Tag::ByteArray(_)) {
                return Err(lua_err(format!("setByteArray: target is {} not byte_array", node.tag.type_name())));
            }
            node.tag = Tag::ByteArray(list.iter().map(|&b| b as i8).collect());
            Ok(())
        });
        methods.add_method("setIntArray", |_, this, (a, b): (Value, Option<Value>)| {
            let (sel, raw) = split_set_args(a, b);
            let t = resolve(this, &sel)?;
            let val = table_arg(&raw, "setIntArray")?;
            let list = signed_list_from_table(&val, i32::MIN as i64, i32::MAX as i64, "setIntArray")?;
            let mut a = this.arena.borrow_mut();
            let node = a.get_mut(t);
            if !matches!(node.tag, Tag::IntArray(_)) {
                return Err(lua_err(format!("setIntArray: target is {} not int_array", node.tag.type_name())));
            }
            node.tag = Tag::IntArray(list.iter().map(|&i| i as i32).collect());
            Ok(())
        });
        methods.add_method("setLongArray", |_, this, (a, b): (Value, Option<Value>)| {
            let (sel, raw) = split_set_args(a, b);
            let t = resolve(this, &sel)?;
            let val = table_arg(&raw, "setLongArray")?;
            let list = signed_list_from_table(&val, i64::MIN, i64::MAX, "setLongArray")?;
            let mut a = this.arena.borrow_mut();
            let node = a.get_mut(t);
            if !matches!(node.tag, Tag::LongArray(_)) {
                return Err(lua_err(format!("setLongArray: target is {} not long_array", node.tag.type_name())));
            }
            node.tag = Tag::LongArray(list);
            Ok(())
        });
        // Container setters replace the (selected) subtree with a copy of
        // another tag's subtree.
        methods.add_method("setCompound", |_, this, (a, b): (Value, Option<AnyUserData>)| {
            let (sel, other) = set_subtree_target(this, a, b, "setCompound")?;
            set_subtree(this, &sel, other)
        });
        methods.add_method("setList", |_, this, (a, b): (Value, Option<AnyUserData>)| {
            let (sel, other) = set_subtree_target(this, a, b, "setList")?;
            set_subtree(this, &sel, other)
        });

        // ---- typed adders: compound `addX(name, v)`, list `addX(v)` --------
        macro_rules! add_scalar {
            ($name:literal, $make:expr) => {
                methods.add_method($name, |_, this, args: MultiValue| {
                    let (name, val) = split_add_args(args)?;
                    let tag = $make(val)?;
                    add_value(this, name, tag)?;
                    Ok(())
                });
            };
        }
        add_scalar!("addByte", |v: Value| match v {
            Value::Boolean(b) => Ok(Tag::Byte(i8::from(b))),
            Value::Integer(i) if (-128..=127).contains(&i) => Ok(Tag::Byte(i as i8)),
            _ => Err(lua_err("addByte: expected an integer in -128..=127")),
        });
        add_scalar!("addBool", |v: Value| match v {
            Value::Boolean(b) => Ok(Tag::Byte(i8::from(b))),
            Value::Integer(i) if i == 0 || i == 1 => Ok(Tag::Byte(i as i8)),
            _ => Err(lua_err("addBool: expected a boolean")),
        });
        add_scalar!("addShort", |v: Value| match v {
            Value::Integer(i) if (-32768..=32767).contains(&i) => Ok(Tag::Short(i as i16)),
            _ => Err(lua_err("addShort: expected an integer in -32768..=32767")),
        });
        add_scalar!("addInt", |v: Value| match v {
            Value::Integer(i) if (i32::MIN as i64..=i32::MAX as i64).contains(&i) => Ok(Tag::Int(i as i32)),
            _ => Err(lua_err("addInt: expected an int32")),
        });
        add_scalar!("addLong", |v: Value| match v {
            Value::Integer(i) => Ok(Tag::Long(i)),
            _ => Err(lua_err("addLong: expected an integer")),
        });
        add_scalar!("addFloat", |v: Value| match v {
            Value::Integer(i) => Ok(Tag::Float(i as f32)),
            Value::Number(n) => Ok(Tag::Float(n as f32)),
            _ => Err(lua_err("addFloat: expected a number")),
        });
        add_scalar!("addDouble", |v: Value| match v {
            Value::Integer(i) => Ok(Tag::Double(i as f64)),
            Value::Number(n) => Ok(Tag::Double(n)),
            _ => Err(lua_err("addDouble: expected a number")),
        });
        add_scalar!("addString", |v: Value| match v {
            Value::String(s) => Ok(Tag::String(s.to_string_lossy())),
            _ => Err(lua_err("addString: expected a string")),
        });
        methods.add_method("addByteArray", |_, this, args: MultiValue| {
            let (name, val) = split_add_args(args)?;
            let t = table_arg(&val, "addByteArray")?;
            let list = signed_list_from_table(&t, -128, 127, "addByteArray")?;
            add_value(this, name, Tag::ByteArray(list.iter().map(|&b| b as i8).collect()))
        });
        methods.add_method("addIntArray", |_, this, args: MultiValue| {
            let (name, val) = split_add_args(args)?;
            let t = table_arg(&val, "addIntArray")?;
            let list = signed_list_from_table(&t, i32::MIN as i64, i32::MAX as i64, "addIntArray")?;
            add_value(this, name, Tag::IntArray(list.iter().map(|&i| i as i32).collect()))
        });
        methods.add_method("addLongArray", |_, this, args: MultiValue| {
            let (name, val) = split_add_args(args)?;
            let t = table_arg(&val, "addLongArray")?;
            let list = signed_list_from_table(&t, i64::MIN, i64::MAX, "addLongArray")?;
            add_value(this, name, Tag::LongArray(list))
        });
        methods.add_method("addCompound", |_, this, args: MultiValue| {
            let (name, val) = split_add_args(args)?;
            let ud = userdata_arg(&val, "addCompound")?;
            add_value(this, name, Tag::Compound(Vec::new()))?;
            copy_children_into(this, &ud, |a, added, src| {
                    a.attach_copy(added, src);
                })
        });
        methods.add_method("addList", |_, this, args: MultiValue| {
            let (name, val) = split_add_args(args)?;
            let ud = userdata_arg(&val, "addList")?;
            add_value(this, name, Tag::List(Vec::new()))?;
            copy_children_into(this, &ud, |a, added, src| {
                    a.attach_copy(added, src);
                })
        });
    }
}

// ---------------------------------------------------------------------------
// Add/set helpers
// ---------------------------------------------------------------------------

fn int_of(v: Value, min: i64, max: i64, what: &str) -> Result<i64, mlua::Error> {
    match v {
        Value::Integer(i) if (min..=max).contains(&i) => Ok(i),
        _ => Err(lua_err(format!("{what}: expected an integer in {min}..={max}"))),
    }
}

/// Type-preserving coercion for `setValue` (the tag's own kind wins).
fn coerce_preserving(tag: &Tag, v: &Value) -> Result<Tag, String> {
    let name = tag.type_name();
    Ok(match tag {
        Tag::Byte(_) => Tag::Byte(int_of(v.clone(), -128, 127, name).map_err(|e| e.to_string())? as i8),
        Tag::Short(_) => Tag::Short(int_of(v.clone(), -32768, 32767, name).map_err(|e| e.to_string())? as i16),
        Tag::Int(_) => Tag::Int(int_of(v.clone(), i64::from(i32::MIN), i64::from(i32::MAX), name).map_err(|e| e.to_string())? as i32),
        Tag::Long(_) => Tag::Long(int_of(v.clone(), i64::MIN, i64::MAX, name).map_err(|e| e.to_string())?),
        Tag::Float(_) => Tag::Float(num_of(v.clone()).map_err(|e| e.to_string())? as f32),
        Tag::Double(_) => Tag::Double(num_of(v.clone()).map_err(|e| e.to_string())?),
        Tag::String(_) => Tag::String(match v {
            Value::String(s) => s.to_string_lossy(),
            _ => return Err(format!("{name} needs a string")),
        }),
        Tag::ByteArray(_) => {
            let t = table_arg(v, name).map_err(|e| e.to_string())?;
            let list = signed_list_from_table(&t, -128, 127, name).map_err(|e| e.to_string())?;
            Tag::ByteArray(list.iter().map(|&b| b as i8).collect())
        }
        Tag::IntArray(_) => {
            let t = table_arg(v, name).map_err(|e| e.to_string())?;
            let list = signed_list_from_table(&t, i64::from(i32::MIN), i64::from(i32::MAX), name).map_err(|e| e.to_string())?;
            Tag::IntArray(list.iter().map(|&i| i as i32).collect())
        }
        Tag::LongArray(_) => {
            let t = table_arg(v, name).map_err(|e| e.to_string())?;
            let list = signed_list_from_table(&t, i64::MIN, i64::MAX, name).map_err(|e| e.to_string())?;
            Tag::LongArray(list)
        }
        Tag::Compound(_) | Tag::List(_) => return Err("setValue cannot replace containers — use addChild/removeChild/clear".into()),
    })
}

fn num_of(v: Value) -> Result<f64, mlua::Error> {
    match v {
        Value::Integer(i) => Ok(i as f64),
        Value::Number(n) => Ok(n),
        _ => Err(lua_err("expected a number")),
    }
}

fn split_add_args(args: MultiValue) -> Result<(Option<String>, Value), mlua::Error> {
    let mut it = args.into_iter();
    match (it.next(), it.next(), it.next()) {
        (Some(Value::String(n)), Some(v), None) => Ok((Some(n.to_string_lossy()), v)),
        (Some(v), None, None) => Ok((None, v)),
        _ => Err(lua_err("expected (name, value) on a compound or (value) on a list")),
    }
}

fn table_arg(v: &Value, what: &str) -> Result<Table, mlua::Error> {
    match v {
        Value::Table(t) => Ok(t.clone()),
        _ => Err(lua_err(format!("{what}: expected a table"))),
    }
}

fn userdata_arg(v: &Value, what: &str) -> Result<LuaTag, mlua::Error> {
    match v {
        Value::UserData(ud) => Ok(ud.borrow::<LuaTag>()?.clone()),
        _ => Err(lua_err(format!("{what}: expected a tag"))),
    }
}

/// Append `value` to this container: named on compounds, appended to lists
/// (homogeneous — mixing element types is a typed error).
fn add_value(this: &LuaTag, name: Option<String>, value: Tag) -> Result<(), mlua::Error> {
    let expected = value.id();
    let mut a = this.arena.borrow_mut();
    {
        let node = a.get(this.idx);
        match &node.tag {
            Tag::Compound(_) => {
                let Some(key) = &name else {
                    return Err(lua_err("a compound child needs a name: addX(name, value)"));
                };
                if node.children.iter().any(|&c| a.get(c).name == *key) {
                    return Err(lua_err(format!("the compound already has a tag named {key:?}")));
                }
            }
            Tag::List(_) => {
                let lt = a.list_type(this.idx);
                if lt != 0 && lt != expected {
                    return Err(lua_err(format!("list elements must share one NBT type ({lt} ≠ {expected})")));
                }
            }
            other => return Err(lua_err(format!("cannot add children to {}", other.type_name()))),
        }
    }
    let idx = a.nodes.len() as u32;
    a.nodes.push(Node { name: name.unwrap_or_default(), tag: value, parent: Some(this.idx), row: -1, path: String::new(), style: RowStyle::default(), children: Vec::new() });
    a.nodes[this.idx as usize].children.push(idx);
    a.renumber(this.idx);
    Ok(())
}

/// After `addCompound`/`addList` created an empty child: pull the source
/// tag's children into it.
fn copy_children_into(this: &LuaTag, src: &LuaTag, mut attach: impl FnMut(&mut Arena, u32, u32)) -> Result<(), mlua::Error> {
    let mut a = this.arena.borrow_mut();
    let added = *a.get(this.idx).children.last().ok_or_else(|| lua_err("internal: added node missing"))?;
    a.get_mut(added).children.clear();
    let src_children = a.get(src.idx).children.clone();
    for c in src_children {
        attach(&mut a, added, c);
    }
    Ok(())
}

/// Resolve `setCompound`/`setList` argument shapes: `t:setX(src)` targets
/// self; `t:setX(sel, src)` targets the selected child.
fn set_subtree_target(this: &LuaTag, a: Value, b: Option<AnyUserData>, what: &str) -> Result<(Option<Value>, LuaTag), mlua::Error> {
    match b {
        Some(ud) => {
            let other = userdata_arg(&Value::UserData(ud), what)?;
            // `a` selects the child; validate it exists.
            resolve(this, &Some(a.clone()))?;
            Ok((Some(a), other))
        }
        None => Ok((None, userdata_arg(&a, what)?)),
    }
}

/// `setCompound`/`setList`: replace this (or the selected) node's subtree
/// with a copy of `other`'s subtree.
fn set_subtree(this: &LuaTag, sel: &Option<Value>, other: LuaTag) -> Result<(), mlua::Error> {
    let t = resolve(this, sel)?;
    let mut a = this.arena.borrow_mut();
    if !matches!(a.get(other.idx).tag, Tag::Compound(_) | Tag::List(_)) {
        return Err(lua_err("setCompound/setList: the source must be a container"));
    }
    let src_tag = a.get(other.idx).tag.clone();
    let src_name = a.get(other.idx).name.clone();
    let src_children = a.get(other.idx).children.clone();
    let children = a.get(t).children.clone();
    for c in children {
        a.detach(c);
    }
    {
        let node = a.get_mut(t);
        node.tag = src_tag;
        node.name = src_name;
        node.children.clear();
    }
    for c in src_children {
        a.attach_copy(t, c);
    }
    Ok(())
}

fn parse_color(v: &Value) -> Result<(u8, u8, u8), mlua::Error> {
    let chan = |n: i64| u8::try_from(n.clamp(0, 255)).unwrap_or(0);
    match v {
        Value::String(s) => {
            let s = s.to_string_lossy();
            let hex = s.strip_prefix('#').unwrap_or(&s);
            if hex.len() == 6 {
                let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| lua_err(format!("setColor: bad color {s:?}")))?;
                let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| lua_err(format!("setColor: bad color {s:?}")))?;
                let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| lua_err(format!("setColor: bad color {s:?}")))?;
                Ok((r, g, b))
            } else {
                Err(lua_err(format!("setColor: expected #rrggbb, got {s:?}")))
            }
        }
        Value::Integer(n) => {
            let n = *n;
            Ok((chan((n >> 16) & 0xff), chan((n >> 8) & 0xff), chan(n & 0xff)))
        }
        Value::Table(t) => {
            let r: i64 = t.get(1)?;
            let g: i64 = t.get(2)?;
            let b: i64 = t.get(3)?;
            Ok((chan(r), chan(g), chan(b)))
        }
        _ => Err(lua_err("setColor: expected \"#rrggbb\", {r,g,b} or 0xRRGGBB")),
    }
}

