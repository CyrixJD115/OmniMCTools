//! The Lua host: globals setup, the tag userdata, and the registration +
//! depth-first walk protocol.
//!
//! Clean-room note: the protocol (register → call every `main` → walk
//! compounds/lists depth-first with 0-based `child(i)` / `childCount`) is
//! reimplemented in Rust from recovered *behavior* (see `EXTENSIONS.md`);
//! exact semantics of unrecovered methods are OMT decisions, documented here.
//!
//! The tree is mirrored into an arena (`Rc<RefCell<Arena>>`) before Lua runs;
//! userdata are handles into it. Styles are collected per node and keyed by
//! the node's original [`omt_nbt::TagPath`] display string; a tree that
//! extensions restructure materializes back through [`NbtStyleRun::edited_root`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};
use omt_nbt::{Tag, TagPath};

use crate::{Context, NbtStyleRun, RowStyle};

/// Node-visit budget per run: extensions run synchronously on the UI thread,
/// so giant documents degrade to "walk truncated" instead of stalling.
const MAX_VISIT: usize = 20_000;

// ---------------------------------------------------------------------------
// Arena
// ---------------------------------------------------------------------------

pub(crate) struct Node {
    pub(crate) name: String,
    /// Authoritative for scalars/arrays; for containers only the *variant*
    /// matters — contents are materialized from `children`.
    pub(crate) tag: Tag,
    pub(crate) parent: Option<u32>,
    /// 0-based position within the parent; −1 for the root and detached nodes.
    pub(crate) row: i64,
    /// Original [`TagPath`] display string ("" for nodes created during the
    /// run — they carry no row identity).
    pub(crate) path: String,
    pub(crate) style: RowStyle,
    pub(crate) children: Vec<u32>,
}

impl Node {
    pub(crate) fn detached(&self) -> Node {
        Node { name: self.name.clone(), tag: self.tag.clone(), parent: None, row: -1, path: String::new(), style: self.style.clone(), children: Vec::new() }
    }
}

pub(crate) struct Arena {
    pub(crate) nodes: Vec<Node>,
}

impl Arena {
    pub(crate) fn build(root_name: &str, root: &Tag) -> Self {
        let mut a = Self { nodes: Vec::new() };
        let mut path = TagPath::root();
        a.build_node(root_name.to_string(), root, None, -1, &mut path);
        a
    }

    pub(crate) fn build_node(&mut self, name: String, tag: &Tag, parent: Option<u32>, row: i64, path: &mut TagPath) -> u32 {
        let idx = self.nodes.len() as u32;
        let path_str = path.to_string();
        self.nodes.push(Node { name, tag: tag.clone(), parent, row, path: path_str, style: RowStyle::default(), children: Vec::new() });
        let child_rows: Vec<(String, &Tag)> = match tag {
            Tag::Compound(entries) => entries.iter().map(|(k, v)| (k.clone(), v)).collect(),
            Tag::List(items) => items.iter().map(|v| (String::new(), v)).collect(),
            _ => Vec::new(),
        };
        for (r, (k, v)) in child_rows.iter().enumerate() {
            if matches!(tag, Tag::Compound(_)) {
                path.push_key_mut(k.clone());
            } else {
                path.push_index_mut(r);
            }
            let child = self.build_node(k.clone(), v, Some(idx), r as i64, path);
            self.nodes[idx as usize].children.push(child);
            path.pop();
        }
        idx
    }

    /// Copy a tag from elsewhere (another document) in as a detached
    /// subtree with no row identity.
    pub(crate) fn import(&mut self, name: String, tag: &Tag) -> u32 {
        let idx = self.nodes.len() as u32;
        self.nodes.push(Node { name, tag: tag.clone(), parent: None, row: -1, path: String::new(), style: RowStyle::default(), children: Vec::new() });
        let kids: Vec<(String, Tag)> = match tag {
            Tag::Compound(e) => e.clone(),
            Tag::List(l) => l.iter().map(|v| (String::new(), v.clone())).collect(),
            _ => Vec::new(),
        };
        for (r, (k, v)) in kids.into_iter().enumerate() {
            let c = self.import(k, &v);
            self.nodes[c as usize].parent = Some(idx);
            self.nodes[c as usize].row = r as i64;
            self.nodes[idx as usize].children.push(c);
        }
        idx
    }

    pub(crate) fn get(&self, idx: u32) -> &Node {
        &self.nodes[idx as usize]
    }

    pub(crate) fn get_mut(&mut self, idx: u32) -> &mut Node {
        &mut self.nodes[idx as usize]
    }

    pub(crate) fn child_count(&self, idx: u32) -> usize {
        self.get(idx).children.len()
    }

    /// Element type id of a list (0 = empty), 0 for non-lists.
    pub(crate) fn list_type(&self, idx: u32) -> u8 {
        if !matches!(self.get(idx).tag, Tag::List(_)) {
            return 0;
        }
        self.get(idx).children.first().map(|&c| self.get(c).tag.id()).unwrap_or(0)
    }

    /// Resolve a selector (`"key"` on compounds, 0-based position on both
    /// container kinds) to a child index.
    pub(crate) fn child_by_sel(&self, idx: u32, sel: &Value) -> Option<u32> {
        let node = self.get(idx);
        match sel {
            Value::String(k) => match &node.tag {
                Tag::Compound(_) => node.children.iter().copied().find(|&c| self.get(c).name == k.to_string_lossy()),
                _ => None,
            },
            Value::Integer(i) if *i >= 0 => node.children.get(*i as usize).copied(),
            _ => None,
        }
    }

    pub(crate) fn renumber(&mut self, parent: u32) {
        let children = self.get(parent).children.clone();
        for (r, c) in children.iter().enumerate() {
            self.nodes[*c as usize].row = r as i64;
        }
    }

    /// Deep-copy `src` (already registered) as a *new* child of `parent`.
    pub(crate) fn attach_copy(&mut self, parent: u32, src: u32) -> u32 {
        let idx = self.copy_subtree(src);
        self.nodes[parent as usize].children.push(idx);
        self.nodes[idx as usize].parent = Some(parent);
        self.renumber(parent);
        idx
    }

    /// Detached deep copy of a subtree (parent None, fresh indices).
    pub(crate) fn copy_subtree(&mut self, src: u32) -> u32 {
        let idx = self.nodes.len() as u32;
        let node = self.get(src).detached();
        self.nodes.push(node);
        let src_children = self.get(src).children.clone();
        for c in src_children {
            let copy = self.copy_subtree(c);
            self.nodes[idx as usize].children.push(copy);
            self.nodes[copy as usize].parent = Some(idx);
        }
        idx
    }

    /// Detach a subtree from its parent (dangling Lua handles become inert).
    pub(crate) fn detach(&mut self, idx: u32) {
        let n = &mut self.nodes[idx as usize];
        n.parent = None;
        n.row = -1;
        n.path.clear();
    }

    pub(crate) fn materialize(&self, idx: u32) -> Tag {
        match &self.get(idx).tag {
            Tag::Compound(_) => Tag::Compound(self.get(idx).children.iter().map(|&c| (self.get(c).name.clone(), self.materialize(c))).collect()),
            Tag::List(_) => Tag::List(self.get(idx).children.iter().map(|&c| self.materialize(c)).collect()),
            other => other.clone(),
        }
    }

    /// Serialized big-endian payload size of this node (id + name excluded).
    pub(crate) fn byte_size(&self, idx: u32) -> usize {
        let nbt = omt_nbt::Nbt { name: String::new(), tag: self.materialize(idx) };
        omt_nbt::write(&nbt, omt_nbt::ByteOrder::Big).map(|b| b.len().saturating_sub(3)).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Tag userdata
// ---------------------------------------------------------------------------

/// Handle into the arena. Cloning is cheap. Lua `==` compares handles, not
/// nodes (two handles for one node are distinct values — OMT-defined).
#[derive(Clone)]
pub(crate) struct LuaTag {
    pub(crate) idx: u32,
    pub(crate) arena: Rc<RefCell<Arena>>,
}

/// `other` as a handle into `this`'s arena: itself when they share one,
/// otherwise a detached copy imported from its own document (tags from
/// `omt.nbt.parse`, world reads, other files…).
pub(crate) fn localize(this: &LuaTag, other: LuaTag) -> LuaTag {
    if Rc::ptr_eq(&this.arena, &other.arena) {
        return other;
    }
    let (name, tag) = {
        let a = other.arena.borrow();
        (a.get(other.idx).name.clone(), a.materialize(other.idx))
    };
    let idx = this.arena.borrow_mut().import(name, &tag);
    LuaTag { idx, arena: this.arena.clone() }
}

pub(crate) fn lua_err(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(msg.into())
}

// ---------------------------------------------------------------------------
// Run
// ---------------------------------------------------------------------------

struct Sub {
    label: String,
    table: Table,
    main: Option<Function>,
    recursion: Option<Function>,
}

fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Nil | Value::Boolean(false))
}

pub(crate) fn run(base: &Path, ids: &[String], root_name: &str, root: &Tag, ctx: &Context) -> NbtStyleRun {
    let mut out = NbtStyleRun::default();
    let arena = Rc::new(RefCell::new(Arena::build(root_name, root)));
    let lua = Lua::new();
    let logs: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));
    if let Err(e) = setup_globals(&lua, base, ctx, logs.clone()) {
        out.errors.push(format!("host setup: {e}"));
        return finalize(arena, root, out);
    }

    // Register: require each enabled extension's main.lua, collect
    // NBT_EDITOR_STYLE sub-extensions (registration order within one
    // main.lua follows Lua `pairs`, as in UMT's bootstrap).
    let mut subs: Vec<Sub> = Vec::new();
    let require: Function = match lua.globals().get("require") {
        Ok(f) => f,
        Err(e) => {
            out.errors.push(format!("host: {e}"));
            return finalize(arena, root, out);
        }
    };
    for id in ids {
        let module: Value = match require.call(format!("{id}.main")) {
            Ok(v) => v,
            Err(e) => {
                out.errors.push(format!("extension {id}: {e}"));
                continue;
            }
        };
        let Value::Table(t) = module else {
            out.errors.push(format!("extension {id}: main.lua must return a table"));
            continue;
        };
        let mut found = 0usize;
        for pair in t.pairs::<Value, Value>() {
            let Ok((_k, v)) = pair else { continue };
            let Value::Table(sub) = v else { continue };
            let ty: Value = sub.get("type").unwrap_or(Value::Nil);
            if !matches!(ty, Value::Integer(n) if n == 1) {
                continue; // not an NBT-editor-style sub-extension
            }
            let main: Option<Function> = sub.get("main").ok().flatten();
            let recursion: Option<Function> = sub.get("recursion").ok().flatten();
            let rec_flag = truthy(&sub.get("recursive").unwrap_or(Value::Nil));
            if main.is_none() && recursion.is_none() {
                continue;
            }
            // Non-recursive sub-extensions never get walk callbacks.
            let recursion = if rec_flag { recursion } else { None };
            subs.push(Sub { label: id.clone(), table: sub, main, recursion });
            found += 1;
        }
        if found == 0 {
            out.logs.push(("info".into(), format!("extension {id}: no NBT_EDITOR_STYLE entries")));
        }
    }

    let ctx_table = match lua.create_table() {
        Ok(t) => t,
        Err(e) => {
            out.errors.push(format!("host: {e}"));
            return finalize(arena, root, out);
        }
    };
    let _ = ctx_table.set("type", i64::from(ctx.file_type));
    let _ = ctx_table.set("edition", ctx.edition.as_str());
    let Ok(root_tag) = lua.create_userdata(LuaTag { idx: 0, arena: arena.clone() }) else {
        out.errors.push("host: cannot create root handle".into());
        return finalize(arena, root, out);
    };

    // Every `main`, in registration order.
    for sub in &subs {
        if let Some(f) = &sub.main
            && let Err(e) = f.call::<()>((sub.table.clone(), root_tag.clone(), ctx_table.clone()))
        {
            out.errors.push(format!("{}: main: {e}", sub.label));
        }
    }

    // Depth-first walk: recursion callbacks run extension-major at each node,
    // then compounds/lists descend over child(0..childCount-1).
    let mut visited = 0usize;
    let mut budget_hit = false;
    walk(&lua, &arena, &subs, &root_tag, &ctx_table, 0, &mut out, &mut visited, &mut budget_hit);
    out.visited = visited;
    if budget_hit {
        out.errors.push(format!("node budget exceeded ({MAX_VISIT}) — extension walk truncated"));
    }
    out.logs = logs.borrow().clone();
    finalize(arena, root, out)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    lua: &Lua,
    arena: &Rc<RefCell<Arena>>,
    subs: &[Sub],
    root_tag: &mlua::AnyUserData,
    ctx_table: &Table,
    idx: u32,
    out: &mut NbtStyleRun,
    visited: &mut usize,
    budget_hit: &mut bool,
) {
    *visited += 1;
    if *visited > MAX_VISIT {
        *budget_hit = true;
        return;
    }
    let ud = match lua.create_userdata(LuaTag { idx, arena: arena.clone() }) {
        Ok(ud) => ud,
        Err(e) => {
            out.errors.push(format!("host: {e}"));
            return;
        }
    };
    for sub in subs {
        if let Some(f) = &sub.recursion
            && let Err(e) = f.call::<()>((sub.table.clone(), root_tag.clone(), ud.clone(), ctx_table.clone()))
        {
            out.errors.push(format!("{}: recursion: {e}", sub.label));
        }
    }
    let children = arena.borrow().get(idx).children.clone();
    for c in children {
        walk(lua, arena, subs, root_tag, ctx_table, c, out, visited, budget_hit);
    }
}

pub(crate) fn setup_globals(lua: &Lua, base: &Path, _ctx: &Context, logs: Rc<RefCell<Vec<(String, String)>>>) -> Result<(), mlua::Error> {
    let g = lua.globals();
    g.set("BASE_PATH", base.display().to_string())?;

    let ty = lua.create_table()?;
    for (i, name) in ["UNDEFINED", "BYTE", "SHORT", "INT", "LONG", "FLOAT", "DOUBLE", "BYTE_ARRAY", "STRING", "LIST", "COMPOUND", "INT_ARRAY", "LONG_ARRAY"]
        .iter()
        .enumerate()
    {
        ty.set(*name, i as i64)?;
    }
    g.set("TYPE", ty)?;

    let ext = lua.create_table()?;
    ext.set("NONE", 0i64)?;
    ext.set("NBT_EDITOR_STYLE", 1i64)?;
    // Extension platform (`omt-extensions`): tool tabs and NBT actions.
    ext.set("TOOL", 2i64)?;
    ext.set("NBT_ACTION", 3i64)?;
    g.set("EXTENSION_TYPE", ext)?;

    let ft = lua.create_table()?;
    for (bit, name) in [(1i64, "CHUNK"), (2, "MAP"), (4, "PLAYER"), (8, "LEVEL"), (16, "GENERIC")] {
        ft.set(name, bit)?;
    }
    g.set("FILE_TYPE", ft)?;

    let ed = lua.create_table()?;
    ed.set("JAVA", 1i64)?;
    ed.set("BEDROCK", 2i64)?;
    g.set("EDITION", ed)?;

    // package.path gains BASE_PATH/?.lua (extension-relative requires);
    // native modules stay disabled (the one hardening measure — see docs).
    let package: Table = g.get("package")?;
    let cur: String = package.get("path")?;
    package.set("path", format!("{}/?.lua;{}", base.display(), cur))?;
    package.set("cpath", "")?;

    // print → run log (keeps GUI stdout clean).
    let print_logs = logs.clone();
    g.set(
        "print",
        lua.create_function(move |_, args: MultiValue| {
            let msg: Vec<String> = args
                .into_iter()
                .map(|v| match v {
                    Value::String(s) => s.to_string_lossy(),
                    Value::Integer(i) => i.to_string(),
                    Value::Number(n) => format!("{n}"),
                    Value::Boolean(b) => b.to_string(),
                    Value::Nil => "nil".into(),
                    other => format!("{other:?}"),
                })
                .collect();
            print_logs.borrow_mut().push(("print".into(), msg.join(" ")));
            Ok(())
        })?,
    )?;

    let logger = lua.create_table()?;
    for level in ["info", "warn", "error"] {
        let sink = logs.clone();
        let lv = level.to_string();
        logger.set(
            level,
            lua.create_function(move |_, args: MultiValue| {
                // Accept both `logger.info(msg)` and `logger:info(msg)` —
                // the colon form passes the logger table as argument #1.
                let msg = match args.into_iter().next_back() {
                    Some(Value::String(s)) => s.to_string_lossy(),
                    Some(other) => format!("{other:?}"),
                    None => String::new(),
                };
                sink.borrow_mut().push((lv.clone(), msg));
                Ok(())
            })?,
        )?;
    }
    g.set("logger", logger)?;
    Ok(())
}

fn finalize(arena: Rc<RefCell<Arena>>, original: &Tag, mut out: NbtStyleRun) -> NbtStyleRun {
    let a = arena.borrow();
    let mut styles = HashMap::new();
    for n in &a.nodes {
        if n.path.is_empty() || n.style.is_empty() {
            continue;
        }
        styles.insert(n.path.clone(), n.style.clone());
    }
    out.styles = styles;
    let new_root = a.materialize(0);
    if new_root != *original {
        out.edited_root = Some(new_root);
    }
    out
}
