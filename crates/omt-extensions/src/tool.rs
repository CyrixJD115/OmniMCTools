//! Running tools and NBT actions.
//!
//! **Discovery** ([`discover`]) loads every enabled extension once and lists
//! its `EXTENSION_TYPE.TOOL` / `NBT_ACTION` entries ([`ToolDecl`]) so the app
//! can show them (sidebar, Map Viewer panel, NBT editor actions).
//!
//! **Hosting** ([`ToolHost`]): each open tool gets its own thread and Lua
//! state. Protocol (all callbacks optional except `view`):
//!
//! ```lua
//! { type = EXTENSION_TYPE.TOOL, id = "chest-explorer", name = "Chest Explorer",
//!   icon = "box", description = "…", map = true,     -- also a Map Viewer panel
//!   init = function(self, ctx) end,                  -- once
//!   view = function(self, ctx) return ui.column{…} end,  -- after every callback
//!   on_event = function(self, ev, ctx) end }         -- clicks, inputs, map clicks…
//! ```
//!
//! `ctx.world` = the world the tool was opened on (or nil), `ctx.node` = the
//! selected tag for NBT actions (edits to it are handed back to the editor).
//! Long callbacks keep the app responsive (worker thread) and stop at the
//! user's Stop (`omt.cancelled()`, plus an instruction-count hook).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender, channel};

use mlua::{AnyUserData, Function, HookTriggers, Lua, Table, Value, VmState};
use omt_nbt::Tag;

use crate::runtime::{Effects, Rt, Shared, WorldEntry, WorldHandle, WorldOpener};
use crate::ui::{self, ToolEvent, UiNode};
use crate::world_api::LuaWorld;

/// Kind of a discovered entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// A tab of its own (sidebar), optionally a Map Viewer panel too.
    Tool,
    /// An action on the NBT editor's selected tag.
    NbtAction,
}

/// A tool / action an installed extension offers.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDecl {
    /// Extension (package) id.
    pub ext: String,
    /// Entry id inside the extension.
    pub id: String,
    pub name: String,
    pub description: String,
    /// UI glyph name (`box`, `map`, `search`, …; unknown → puzzle piece).
    pub icon: String,
    pub kind: ToolKind,
    /// Also offered as a Map Viewer side panel (receives map clicks).
    pub map: bool,
    /// NBT actions: tag names they apply to (empty = any).
    pub names: Vec<String>,
    /// NBT actions: tag type ids they apply to (empty = any).
    pub types: Vec<u8>,
}

impl ToolDecl {
    /// Stable key (`ext/id`).
    pub fn key(&self) -> String {
        format!("{}/{}", self.ext, self.id)
    }

    /// Whether an NBT action applies to a tag named `name` of type `ty`.
    pub fn applies(&self, name: &str, ty: u8) -> bool {
        self.kind == ToolKind::NbtAction && (self.names.is_empty() || self.names.iter().any(|n| n == name)) && (self.types.is_empty() || self.types.contains(&ty))
    }
}

fn str_field(t: &Table, k: &str) -> String {
    t.get::<Option<String>>(k).ok().flatten().unwrap_or_default()
}

/// A Lua state with the full extension environment.
fn new_state(root: &Path, rt: Rt) -> mlua::Result<(Lua, omt_script::lua::Logs)> {
    let lua = Lua::new();
    let logs = omt_script::lua::Logs::default();
    omt_script::lua::install_globals(&lua, root, logs.clone())?;
    lua.load(ui::PRELUDE).set_name("ui prelude").exec()?;
    lua.set_app_data(rt);
    crate::api::install(&lua)?;
    Ok((lua, logs))
}

fn bare_rt(root: &Path, ext: &str) -> Rt {
    Rt { shared: Arc::new(Shared::default()), world: None, opener: None, library: Vec::new(), ext_dir: root.join(ext), effects: Effects::default() }
}

/// Entries of `main.lua`'s returned table with `type == want`, by id.
fn entries(lua: &Lua, ext: &str, want: &[i64]) -> mlua::Result<Vec<(String, Table)>> {
    let require: Function = lua.globals().get("require")?;
    let module: Value = require.call(format!("{ext}.main"))?;
    let Value::Table(t) = module else { return Err(mlua::Error::runtime("main.lua must return a table")) };
    let mut out = Vec::new();
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let Value::Table(sub) = v else { continue };
        let Ok(Value::Integer(ty)) = sub.get::<Value>("type") else { continue };
        if !want.contains(&ty) {
            continue;
        }
        let id = Some(str_field(&sub, "id")).filter(|s| !s.is_empty()).unwrap_or_else(|| match k {
            Value::String(s) => s.to_string_lossy(),
            other => other.to_string().unwrap_or_default(),
        });
        out.push((id, sub));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// List the tools and NBT actions of the given (enabled) extensions.
/// Broken extensions are reported, never fatal.
pub fn discover(root: &Path, ids: &[String]) -> (Vec<ToolDecl>, Vec<String>) {
    let (mut decls, mut errors) = (Vec::new(), Vec::new());
    for ext in ids {
        let r = (|| -> mlua::Result<()> {
            let (lua, _) = new_state(root, bare_rt(root, ext))?;
            for (id, t) in entries(&lua, ext, &[2, 3])? {
                let kind = if t.get::<i64>("type")? == 2 { ToolKind::Tool } else { ToolKind::NbtAction };
                let list = |k: &str| -> Vec<Value> { t.get::<Option<Table>>(k).ok().flatten().map(|l| l.sequence_values::<Value>().filter_map(Result::ok).collect()).unwrap_or_default() };
                let names = list("names").into_iter().filter_map(|v| v.as_string().map(|s| s.to_string_lossy())).collect();
                let types = list("types").into_iter().filter_map(|v| v.as_integer().map(|i| i as u8)).collect();
                decls.push(ToolDecl {
                    ext: ext.clone(),
                    name: Some(str_field(&t, "name")).filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone()),
                    description: str_field(&t, "description"),
                    icon: str_field(&t, "icon"),
                    map: t.get::<Option<bool>>("map")?.unwrap_or(false),
                    kind,
                    names,
                    types,
                    id,
                });
            }
            Ok(())
        })();
        if let Err(e) = r {
            errors.push(format!("{ext}: {e}"));
        }
    }
    (decls, errors)
}

/// What a tool shows after a callback.
#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
    pub title: String,
    /// `None` when `view` failed (keep showing the previous one).
    pub view: Option<UiNode>,
    /// Lua error of this callback, if any.
    pub error: Option<String>,
    pub effects: Effects,
}

/// Everything a tool starts with.
#[derive(Clone, Default)]
pub struct ToolSpec {
    /// Extensions folder.
    pub root: PathBuf,
    pub ext: String,
    pub tool: String,
    pub world: Option<WorldHandle>,
    pub opener: Option<WorldOpener>,
    pub library: Vec<WorldEntry>,
    /// NBT action target: (name, tag).
    pub node: Option<(String, Tag)>,
}

enum Req {
    Event(ToolEvent, Sender<ToolOutput>),
    Refresh(Sender<ToolOutput>),
}

/// A running tool (its worker thread ends when this is dropped).
pub struct ToolHost {
    tx: Sender<Req>,
    pub shared: Arc<Shared>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl ToolHost {
    /// Start the tool; the receiver yields the first view (after `init`).
    pub fn start(spec: ToolSpec) -> (Self, Receiver<ToolOutput>) {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = channel::<Req>();
        let (first_tx, first_rx) = channel();
        let sh = Arc::clone(&shared);
        let join = std::thread::Builder::new().name(format!("ext-{}-{}", spec.ext, spec.tool)).spawn(move || worker(spec, sh, rx, first_tx)).ok();
        (Self { tx, shared, join }, first_rx)
    }

    /// Stop the tool and wait for its thread (its Lua state — and every
    /// world handle it holds — is gone when this returns). A running
    /// callback is cancelled first.
    pub fn shutdown(self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
        let Self { tx, join, .. } = self;
        drop(tx);
        if let Some(j) = join {
            let _ = j.join();
        }
    }

    /// Deliver an event; the receiver yields the resulting view.
    pub fn send(&self, ev: ToolEvent) -> Receiver<ToolOutput> {
        let (tx, rx) = channel();
        self.shared.cancel.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Req::Event(ev, tx));
        rx
    }

    /// Re-run `view` (e.g. after the app changed something).
    pub fn refresh(&self) -> Receiver<ToolOutput> {
        let (tx, rx) = channel();
        let _ = self.tx.send(Req::Refresh(tx));
        rx
    }

    /// Ask the running callback to stop.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
}

/// A running tool, abstracted over its backend (Lua [`ToolHost`] or a
/// native plugin host). The GUI drives every open tool through this.
pub trait ToolBackend: Send {
    /// Deliver a UI event; the receiver yields the resulting view.
    fn send(&self, ev: ToolEvent) -> Receiver<ToolOutput>;
    /// Re-render (the app changed something).
    fn refresh(&self) -> Receiver<ToolOutput>;
    /// Ask the running callback to stop.
    fn cancel(&self);
    /// Live progress / busy state.
    fn shared(&self) -> &Arc<Shared>;
    /// Stop and join the worker (its world handles are gone on return).
    fn shutdown(self: Box<Self>);
}

impl ToolBackend for ToolHost {
    fn send(&self, ev: ToolEvent) -> Receiver<ToolOutput> {
        ToolHost::send(self, ev)
    }
    fn refresh(&self) -> Receiver<ToolOutput> {
        ToolHost::refresh(self)
    }
    fn cancel(&self) {
        ToolHost::cancel(self);
    }
    fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
    fn shutdown(self: Box<Self>) {
        ToolHost::shutdown(*self);
    }
}

struct Live {
    lua: Lua,
    logs: omt_script::lua::Logs,
    me: Table,
    ctx: Table,
    node: Option<AnyUserData>,
    /// Last node value handed back (only changes are reported).
    node_seen: Option<Tag>,
}

impl Live {
    /// Run `f` (a callback by name) then `view`, collecting effects.
    fn call(&mut self, shared: &Shared, name: &str, ev: Option<&ToolEvent>) -> ToolOutput {
        shared.busy.store(true, Ordering::Relaxed);
        let mut out = ToolOutput::default();
        let run = (|| -> mlua::Result<()> {
            if let Some(f) = self.me.get::<Option<Function>>(name)? {
                match ev {
                    Some(ev) => f.call::<()>((self.me.clone(), ev.to_lua(&self.lua)?, self.ctx.clone()))?,
                    None => f.call::<()>((self.me.clone(), self.ctx.clone()))?,
                }
            }
            Ok(())
        })();
        if let Err(e) = run {
            out.error = Some(format!("{name}: {e}"));
        }
        let view = (|| -> mlua::Result<Option<UiNode>> {
            let f: Option<Function> = self.me.get("view")?;
            let Some(f) = f else { return Ok(Some(UiNode::Text { text: "This tool has no view() function.".into(), size: 13.0, color: None, bold: false, mono: false })) };
            let v: Value = f.call((self.me.clone(), self.ctx.clone()))?;
            ui::parse_value(&v, 0)
        })();
        match view {
            Ok(v) => out.view = v.or(Some(UiNode::Column { children: Vec::new(), spacing: 0.0 })),
            Err(e) => out.error = Some(out.error.take().map_or(format!("view: {e}"), |p| format!("{p}\nview: {e}"))),
        }
        out.title = str_field(&self.me, "title");
        if out.title.is_empty() {
            out.title = str_field(&self.me, "name");
        }
        if let Some(mut rt) = self.lua.app_data_mut::<Rt>() {
            out.effects = std::mem::take(&mut rt.effects);
            rt.shared.set_progress(None);
        }
        out.effects.logs.extend(self.logs.borrow_mut().drain(..));
        if let Some(n) = &self.node
            && let Ok((_, t)) = omt_script::lua::tag_of(n)
            && self.node_seen.as_ref() != Some(&t)
        {
            self.node_seen = Some(t.clone());
            out.effects.replace = Some(t);
        }
        shared.busy.store(false, Ordering::Relaxed);
        out
    }
}

fn start_live(spec: &ToolSpec, shared: &Arc<Shared>) -> Result<Live, String> {
    let rt = Rt { shared: Arc::clone(shared), world: spec.world.clone(), opener: spec.opener.clone(), library: spec.library.clone(), ext_dir: spec.root.join(&spec.ext), effects: Effects::default() };
    let (lua, logs) = new_state(&spec.root, rt).map_err(|e| e.to_string())?;
    // Stop button: abort runaway Lua between instructions.
    let sh = Arc::clone(shared);
    lua.set_hook(HookTriggers::new().every_nth_instruction(20_000), move |_, _| if sh.cancelled() { Err(mlua::Error::runtime("cancelled")) } else { Ok(VmState::Continue) }).map_err(|e| e.to_string())?;
    let me = entries(&lua, &spec.ext, &[2, 3]).map_err(|e| e.to_string())?.into_iter().find(|(id, _)| *id == spec.tool).map(|(_, t)| t).ok_or_else(|| format!("{} has no tool {:?}", spec.ext, spec.tool))?;
    let ctx = lua.create_table().map_err(|e| e.to_string())?;
    let mut node = None;
    (|| -> mlua::Result<()> {
        if let Some(w) = &spec.world {
            ctx.set("world", lua.create_userdata(LuaWorld::new(w.clone()))?)?;
            ctx.set("edition", if w.edition == omt_world::Edition::Java { "JAVA" } else { "BEDROCK" })?;
        }
        if let Some((name, t)) = &spec.node {
            let ud = omt_script::lua::new_tag(&lua, name, t)?;
            ctx.set("node", ud.clone())?;
            node = Some(ud);
        }
        Ok(())
    })()
    .map_err(|e| e.to_string())?;
    Ok(Live { node_seen: spec.node.as_ref().map(|(_, t)| t.clone()), lua, logs, me, ctx, node })
}

fn worker(spec: ToolSpec, shared: Arc<Shared>, rx: Receiver<Req>, first: Sender<ToolOutput>) {
    let mut live = match start_live(&spec, &shared) {
        Ok(l) => l,
        Err(e) => {
            let _ = first.send(ToolOutput { title: spec.tool.clone(), error: Some(e), ..Default::default() });
            return;
        }
    };
    let _ = first.send(live.call(&shared, "init", None));
    while let Ok(req) = rx.recv() {
        match req {
            Req::Event(ev, reply) => {
                let _ = reply.send(live.call(&shared, "on_event", Some(&ev)));
            }
            Req::Refresh(reply) => {
                let _ = reply.send(live.call(&shared, "", None));
            }
        }
    }
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
