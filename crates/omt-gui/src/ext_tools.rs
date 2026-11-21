//! Extension **tool tabs** (`omt-extensions`): discovery of the tools and
//! NBT actions installed extensions offer, opening a tool on the current
//! world (its own worker thread), routing UI events to it, and applying its
//! effects (status notes, clipboard, "session modified").

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::{button, column, container, progress_bar, row, scrollable, space, text};
use iced::{Alignment, Element, Length, Task};
use omt_extensions::runtime::{WorldEntry, WorldHandle, WorldOpener};
use omt_extensions::tool::{ToolBackend, ToolDecl, ToolHost, ToolKind, ToolOutput, ToolSpec};
use omt_extensions::{Pick, ToolEvent, UiNode};

use crate::session::{Store, blocking};
use crate::style::{self, Btn};
use crate::{Message, OmniMCTools, Page};

#[derive(Debug, Clone)]
pub enum ExtToolMsg {
    /// Open (or focus) a tool by `ext/id`.
    Open(String),
    Close(u64),
    Event(u64, ToolEvent),
    /// Text typed into an input (shown at once, sent to the tool).
    Typed(u64, String, String),
    Submit(u64, String),
    /// Slider moved (local) / released (sent).
    Dragged(u64, String, f64),
    Released(u64, String),
    /// A `pick` button: show the dialog, then deliver the path.
    Pick(u64, String, Pick),
    Picked(u64, String, Option<PathBuf>),
    /// A callback finished (tool uid, request sequence number).
    Output(u64, u64, Box<ToolOutput>),
    Stop(u64),
    /// Progress refresh while a callback runs.
    Tick,
    /// Restart a tool from its files on disk (after editing main.lua).
    Reload(u64),
    /// Open a tool without leaving the current page (Map Viewer panel).
    OpenHere(String),
    /// A Map Viewer click (world origin, block x / z, dimension).
    MapClick(PathBuf, i32, i32, u8),
    /// Discovery result for a manager revision.
    Discovered(u64, Vec<ToolDecl>, Vec<String>),
}

/// One open tool tab.
pub struct OpenTool {
    pub uid: u64,
    pub decl: ToolDecl,
    /// `None` while suspended (its world session is saving).
    /// The running backend (Lua `ToolHost` or a native plugin host).
    host: Option<Box<dyn ToolBackend>>,
    /// What it was started with (Reload restarts it from fresh files).
    spec: ToolSpec,
    pub title: String,
    pub view: Option<UiNode>,
    pub error: Option<String>,
    /// Requests in flight.
    pub pending: usize,
    seq_sent: u64,
    seq_shown: u64,
    /// Locally typed / dragged values not yet echoed by the tool.
    pub overrides: HashMap<String, String>,
    /// The world it runs on (label) and its session (for "modified").
    pub world_label: Option<String>,
    session_origin: Option<PathBuf>,
    pub logs: Vec<(String, String)>,
    /// NBT actions: the editor tab (uid) and tag path its edits go to.
    target: Option<(u64, omt_nbt::TagPath)>,
}

/// Tools of the installed extensions + the open tool tabs.
#[derive(Default)]
pub struct ExtHub {
    pub decls: Vec<ToolDecl>,
    pub errors: Vec<String>,
    /// Manager revision the decls were discovered for.
    rev: Option<u64>,
    pub open: Vec<OpenTool>,
    next_uid: u64,
    /// The world session last on screen (tools open on it).
    pub last_session: Option<PathBuf>,
    /// Library worlds for world pickers: (name, folder).
    pub library: Vec<(String, String)>,
    /// `OMT_DEBUG_OPEN_TOOL` already applied.
    debug_opened: bool,
    /// Bumped per discovery (editor tabs refresh their action lists).
    pub generation: u64,
}

impl ExtHub {
    pub fn tool(&self, uid: u64) -> Option<&OpenTool> {
        self.open.iter().find(|t| t.uid == uid)
    }

    fn tool_mut(&mut self, uid: u64) -> Option<&mut OpenTool> {
        self.open.iter_mut().find(|t| t.uid == uid)
    }

    pub fn busy(&self) -> bool {
        self.open.iter().any(|t| t.pending > 0)
    }
}

/// Extension writes through `omt-edit` (the World Editor's verified
/// per-edition writers: Bedrock records, Java region files, atomic).
struct EditWriter(omt_edit::EditWorld);

impl omt_extensions::runtime::WorldWriter for EditWriter {
    fn set_block_entities(&self, dim: u8, cx: i32, cz: i32, list: &[omt_nbt::Tag]) -> Result<(), String> {
        omt_edit::block_entities::set_block_entities(&self.0, dim, cx, cz, list).map(|_| ()).map_err(|e| e.to_string())
    }

    fn fill(&self, dim: u8, a: [i32; 3], b: [i32; 3], block: &str, replace: Option<&str>) -> Result<u64, String> {
        let m = omt_edit::Matcher::parse(block);
        let to = omt_edit::Block { state: omt_world::BlockState { name: m.name, properties: m.props }, bedrock_raw: None };
        let op = match replace {
            Some(r) => omt_edit::Op::Replace { from: omt_edit::Matcher::parse(r), to },
            None => omt_edit::Op::Fill { to, only_air: false },
        };
        omt_edit::apply(&self.0, dim, omt_edit::Sel::new(a, b), &op, &|| false).map(|(r, _)| r.changed).map_err(|e| e.to_string())
    }
}

/// A store as the trait object extensions use (`writable` = an editable
/// session: writes land in its workspace, the session's Save keeps them).
pub fn world_handle(store: &Store, path: PathBuf, name: String, writable: bool) -> WorldHandle {
    let (store, edition, edit): (Arc<dyn omt_world::WorldStore>, _, _) = match store {
        Store::Java(w) => (w.clone(), omt_world::Edition::Java, omt_edit::EditWorld::Java(w.clone())),
        Store::Bedrock(w) => (w.clone(), omt_world::Edition::Bedrock, omt_edit::EditWorld::Bedrock(w.clone())),
    };
    let writer = writable.then(|| Arc::new(EditWriter(edit)) as Arc<dyn omt_extensions::runtime::WorldWriter>);
    WorldHandle { store, edition, path, name, writable, writer }
}

/// Opens other worlds for `omt.world.open` (read-only, in place).
pub fn opener() -> WorldOpener {
    Arc::new(|p: &std::path::Path| {
        let store = Store::open_read_only(p)?;
        let name = omt_world::detect(p).ok().and_then(|d| omt_world::WorldSummary::from_detected(&d).ok()).map_or_else(|| p.display().to_string(), |s| s.name);
        Ok(world_handle(&store, p.to_path_buf(), name, false))
    })
}

/// Start the right backend for a tool spec: a native plugin host when
/// `native`, otherwise the Lua `ToolHost`. Both yield a `ToolOutput` stream.
fn start_backend(native: bool, spec: ToolSpec) -> (Box<dyn ToolBackend>, std::sync::mpsc::Receiver<ToolOutput>) {
    if native {
        let (h, rx) = omt_nativehost::NativeHost::start(spec);
        (Box::new(h), rx)
    } else {
        let (h, rx) = ToolHost::start(spec);
        (Box::new(h), rx)
    }
}

fn wait(uid: u64, seq: u64, rx: std::sync::mpsc::Receiver<ToolOutput>) -> Task<Message> {
    Task::perform(blocking(move || rx.recv().unwrap_or_else(|_| ToolOutput { error: Some("the tool stopped".into()), ..Default::default() })), move |o| Message::ExtTool(ExtToolMsg::Output(uid, seq, Box::new(o))))
}

impl OmniMCTools {
    /// Re-discover tools when the extension set changed.
    pub(crate) fn sync_ext_tools(&mut self) -> Task<Message> {
        if let Page::Session(i) = self.screen
            && let Some(o) = self.sessions.get(i).and_then(|s| s.sess.as_ref()).map(|s| s.origin())
        {
            self.hub.last_session = Some(o);
        }
        if self.hub.library.len() != self.worlds.len() {
            self.hub.library = self.worlds.iter().map(|w| (w.summary.name.clone(), w.origin.display().to_string())).collect();
        }
        // Master switch off: no tools, no NBT actions, no hooks (styles are
        // gated inside `Manager::run`). The revision check below re-runs
        // discovery as soon as it turns back on.
        if !self.ext.master() {
            if !self.hub.decls.is_empty() || !self.hub.errors.is_empty() {
                self.hub.decls.clear();
                self.hub.errors.clear();
                self.hub.generation += 1;
            }
            self.hub.rev = None;
            return Task::none();
        }
        // NBT actions: keep every editor tab's list current, and open the
        // actions the user started.
        let generation = self.hub.generation;
        let actions: Vec<ToolDecl> = self.hub.decls.iter().filter(|d| d.kind == ToolKind::NbtAction).cloned().collect();
        let mut started = Vec::new();
        // Debug/testing hook (`OMT_DEBUG_NBT_ACTION="<tag name>=<ext>/<id>"`):
        // select the first tag of that name and run the action (once).
        let debug_action = (!self.hub.debug_opened && !actions.is_empty()).then(|| std::env::var("OMT_DEBUG_NBT_ACTION").ok()).flatten();
        for et in self.editor_tabs() {
            if et.actions_gen != generation {
                et.actions = actions.clone();
                et.actions_gen = generation;
            }
            if et.actions_gen == generation
                && let Some((name, key)) = debug_action.as_deref().and_then(|d| d.split_once('='))
                && let Some(path) = et.rows.iter().find(|r| r.label == name).map(|r| r.path.clone())
            {
                et.selected = Some(path.clone());
                et.expand(&path);
                et.run_action(key.to_string());
            }
            if let Some(p) = et.pending_action.take() {
                started.push((et.uid, p));
            }
        }
        if debug_action.is_some() && !started.is_empty() {
            self.hub.debug_opened = true;
        }
        let mut opened = Vec::new();
        for (tab, (key, path, name, tag)) in started {
            let Some(decl) = self.hub.decls.iter().find(|d| d.key() == key).cloned() else { continue };
            let title = format!("{} — {name}", decl.name);
            opened.push(self.open_tool(decl, Some((name, tag)), true));
            let uid = self.hub.next_uid;
            if let Some(t) = self.hub.open.last_mut() {
                t.target = Some((tab, path));
                t.title = title;
            }
            let events = self.debug_event_tasks(uid);
            if !events.is_empty()
                && let Some(i) = self.docs.iter().position(|d| d.uid == tab)
            {
                // Debug replay: show the editor so the applied edit is visible.
                opened.push(Task::done(Message::Nav(Page::Doc(i))));
            }
            opened.extend(events);
        }
        if !opened.is_empty() {
            return Task::batch(opened);
        }
        // Debug/testing hook (`OMT_DEBUG_OPEN_TOOL=<ext>/<tool>`, with
        // `OMT_DEBUG_TOOL_EVENTS="submit:search:diamond;slot:chest:0;click:tp"`):
        // open a tool on the first world session, then replay events.
        if !self.hub.debug_opened
            && let Ok(key) = std::env::var("OMT_DEBUG_OPEN_TOOL")
            && self.sessions.iter().any(|s| s.sess.is_some())
            && let Some(decl) = self.hub.decls.iter().find(|d| d.key() == key).cloned()
        {
            self.hub.debug_opened = true;
            if self.hub.last_session.is_none() {
                self.hub.last_session = self.sessions.iter().find_map(|s| s.sess.as_ref()).map(|s| s.origin());
            }
            // `OMT_DEBUG_TOOL_PANEL=1`: open it in the Map Viewer's panel instead.
            let panel = std::env::var_os("OMT_DEBUG_TOOL_PANEL").is_some();
            let open = self.open_tool(decl, None, !panel);
            let uid = self.hub.next_uid;
            let open = match (panel, self.sessions.iter().position(|s| matches!(s.ui, crate::Ui::Viewer(_)))) {
                (true, Some(i)) => {
                    self.screen = Page::Session(i);
                    Task::batch([open, Task::done(Message::View(i, crate::viewer::ViewerMsg::Panel(Some(crate::viewer::PanelTab::Tools))))])
                }
                _ => open,
            };
            let events = self.debug_event_tasks(uid);
            return Task::batch(std::iter::once(open).chain(events));
        }
        let rev = self.ext.revision();
        if self.hub.rev == Some(rev) {
            return Task::none();
        }
        self.hub.rev = Some(rev);
        let root = self.ext.root().to_path_buf();
        let records = self.ext.records();
        let enabled = |native: bool| -> Vec<String> { records.iter().filter(|r| r.native == native && self.ext.is_enabled(&r.id)).map(|r| r.id.clone()).collect() };
        let (lua_ids, native_ids) = (enabled(false), enabled(true));
        Task::perform(
            blocking(move || {
                let (mut d, mut e) = omt_extensions::tool::discover(&root, &lua_ids);
                let (nd, ne) = omt_nativehost::discover_native(&root, &native_ids);
                d.extend(nd);
                e.extend(ne);
                (d, e)
            }),
            move |(d, e)| Message::ExtTool(ExtToolMsg::Discovered(rev, d, e)),
        )
    }

    /// Every NBT editor tab (standalone files and NBT Editor sessions).
    fn editor_tabs(&mut self) -> Vec<&mut crate::editor::EditorTab> {
        let mut out: Vec<&mut crate::editor::EditorTab> = self.docs.iter_mut().collect();
        for s in &mut self.sessions {
            if let crate::Ui::Nbt(n) = &mut s.ui {
                out.extend(n.docs_mut().iter_mut());
            }
        }
        out
    }

    /// Before a session saves: stop its tools (they hold the world store,
    /// which the save must close and reopen).
    pub(crate) fn suspend_tools(&mut self, origin: &std::path::Path) {
        for t in self.hub.open.iter_mut().filter(|t| t.session_origin.as_deref() == Some(origin)) {
            if let Some(h) = t.host.take() {
                h.shutdown();
            }
            t.spec.world = None;
            t.pending = 0;
        }
    }

    /// After the save: restart them on the reopened store (they run `init`
    /// again — keep lasting state in `omt.storage`).
    pub(crate) fn resume_tools(&mut self, origin: &std::path::Path) -> Task<Message> {
        let world = self.world_for(origin);
        let native = self.native_exts();
        let mut tasks = Vec::new();
        for t in self.hub.open.iter_mut().filter(|t| t.session_origin.as_deref() == Some(origin) && t.host.is_none()) {
            t.spec.world = world.clone();
            let (host, rx) = start_backend(native.contains(&t.spec.ext), t.spec.clone());
            t.host = Some(host);
            t.seq_sent += 1;
            t.seq_shown = t.seq_sent;
            t.pending += 1;
            tasks.push(wait(t.uid, t.seq_sent, rx));
        }
        Task::batch(tasks)
    }

    /// Whether an extension folder is a native plugin (a dynamic library).
    fn ext_is_native(&self, ext: &str) -> bool {
        self.ext.records().iter().any(|r| r.id == ext && r.native)
    }

    /// The ids of every native-plugin extension folder.
    fn native_exts(&self) -> std::collections::HashSet<String> {
        self.ext.records().iter().filter(|r| r.native).map(|r| r.id.clone()).collect()
    }

    /// A world handle for the open session at `origin`.
    fn world_for(&self, origin: &std::path::Path) -> Option<WorldHandle> {
        let s = self.sessions.iter().filter_map(|t| t.sess.as_ref()).find(|s| s.origin() == origin)?;
        let path = s.workspace.as_ref().map_or_else(|| origin.to_path_buf(), |w| w.root().to_path_buf());
        Some(world_handle(s.store.as_ref()?, path, s.title.clone(), !s.in_place))
    }

    /// The world a tool opens on: the session on screen, else the last one.
    fn tool_world(&self) -> Option<(WorldHandle, PathBuf)> {
        let origin = match self.screen {
            Page::Session(i) => self.sessions.get(i).and_then(|s| s.sess.as_ref()).map(|s| s.origin()),
            _ => self.hub.last_session.clone(),
        }?;
        let s = self.sessions.iter().filter_map(|t| t.sess.as_ref()).find(|s| s.origin() == origin)?;
        let store = s.store.as_ref()?;
        let path = s.workspace.as_ref().map_or_else(|| origin.clone(), |w| w.root().to_path_buf());
        Some((world_handle(store, path, s.title.clone(), !s.in_place), origin))
    }

    pub(crate) fn on_ext_tool(&mut self, m: ExtToolMsg) -> Task<Message> {
        match m {
            ExtToolMsg::Discovered(rev, d, e) => {
                if self.hub.rev == Some(rev) {
                    self.hub.decls = d;
                    self.hub.errors = e;
                    self.hub.generation += 1;
                }
            }
            ExtToolMsg::Open(key) => {
                // One tab per tool: focus it when open.
                if let Some(t) = self.hub.open.iter().find(|t| t.decl.key() == key) {
                    self.screen = Page::ExtTool(t.uid);
                    return Task::none();
                }
                let Some(decl) = self.hub.decls.iter().find(|d| d.key() == key && d.kind == ToolKind::Tool).cloned() else { return Task::none() };
                return self.open_tool(decl, None, true);
            }
            ExtToolMsg::OpenHere(key) => {
                let Some(decl) = self.hub.decls.iter().find(|d| d.key() == key && d.kind == ToolKind::Tool).cloned() else { return Task::none() };
                return self.open_tool(decl, None, false);
            }
            ExtToolMsg::MapClick(origin, x, z, dim) => {
                let uid = self.hub.open.iter().find(|t| t.decl.map && t.session_origin.as_ref() == Some(&origin)).map(|t| t.uid);
                if let Some(uid) = uid {
                    return self.send_tool(uid, ToolEvent::MapClick { x, y: None, z, dim });
                }
            }
            ExtToolMsg::Close(uid) => {
                self.hub.open.retain(|t| t.uid != uid);
                if self.screen == Page::ExtTool(uid) {
                    self.screen = Page::Home;
                }
            }
            ExtToolMsg::Event(uid, ev) => return self.send_tool(uid, ev),
            ExtToolMsg::Typed(uid, id, v) => {
                if let Some(t) = self.hub.tool_mut(uid) {
                    t.overrides.insert(id.clone(), v.clone());
                }
                return self.send_tool(uid, ToolEvent::Input { id, value: v });
            }
            ExtToolMsg::Submit(uid, id) => {
                let value = self.hub.tool_mut(uid).and_then(|t| t.overrides.remove(&id)).or_else(|| self.hub.tool(uid).and_then(|t| t.view.as_ref()).and_then(|v| input_value(v, &id))).unwrap_or_default();
                return self.send_tool(uid, ToolEvent::Submit { id, value });
            }
            ExtToolMsg::Dragged(uid, id, v) => {
                if let Some(t) = self.hub.tool_mut(uid) {
                    t.overrides.insert(id, v.to_string());
                }
            }
            ExtToolMsg::Released(uid, id) => {
                let v = self.hub.tool(uid).and_then(|t| t.overrides.get(&id)).and_then(|v| v.parse().ok());
                if let Some(value) = v {
                    return self.send_tool(uid, ToolEvent::Slider { id, value });
                }
            }
            ExtToolMsg::Pick(uid, id, pick) => {
                return Task::perform(
                    async move {
                        let d = rfd::AsyncFileDialog::new();
                        let h = match pick {
                            Pick::Folder | Pick::World => d.pick_folder().await,
                            Pick::SaveFile => d.save_file().await,
                            _ => d.pick_file().await,
                        };
                        h.map(|h| h.path().to_path_buf())
                    },
                    move |p| Message::ExtTool(ExtToolMsg::Picked(uid, id.clone(), p)),
                );
            }
            ExtToolMsg::Picked(uid, id, Some(p)) => return self.send_tool(uid, ToolEvent::Picked { id, path: p.display().to_string() }),
            ExtToolMsg::Picked(_, _, None) => {}
            ExtToolMsg::Output(uid, seq, out) => return self.tool_output(uid, seq, *out),
            ExtToolMsg::Reload(uid) => {
                // Re-read the extension set too (new / renamed entries).
                self.ext.reload();
                let Some(ext) = self.hub.tool(uid).map(|t| t.spec.ext.clone()) else { return Task::none() };
                let native = self.ext_is_native(&ext);
                let Some(t) = self.hub.tool_mut(uid) else { return Task::none() };
                if let Some(h) = t.host.take() {
                    h.shutdown();
                }
                let (host, rx) = start_backend(native, t.spec.clone());
                t.host = Some(host);
                t.view = None;
                t.error = None;
                t.logs.clear();
                t.overrides.clear();
                t.seq_sent += 1;
                t.pending += 1;
                let seq = t.seq_sent;
                t.seq_shown = seq;
                self.status = format!("{}: reloaded from {}", t.title, self.ext.root().join(&t.decl.ext).display());
                return wait(uid, seq, rx);
            }
            ExtToolMsg::Stop(uid) => {
                if let Some(h) = self.hub.tool(uid).and_then(|t| t.host.as_ref()) {
                    h.cancel();
                }
            }
            ExtToolMsg::Tick => {}
        }
        Task::none()
    }

    /// Start a tool (NBT actions pass the selected tag as `node`).
    pub(crate) fn open_tool(&mut self, decl: ToolDecl, node: Option<(String, omt_nbt::Tag)>, focus: bool) -> Task<Message> {
        let (world, origin) = self.tool_world().map_or((None, None), |(w, o)| (Some(w), Some(o)));
        let library = self.worlds.iter().map(|w| WorldEntry { name: w.summary.name.clone(), path: w.origin.clone(), edition: w.summary.edition }).collect();
        let spec = ToolSpec { root: self.ext.root().to_path_buf(), ext: decl.ext.clone(), tool: decl.id.clone(), world: world.clone(), opener: Some(opener()), library, node };
        let (host, rx) = start_backend(self.ext_is_native(&decl.ext), spec.clone());
        self.hub.next_uid += 1;
        let uid = self.hub.next_uid;
        self.hub.open.push(OpenTool {
            uid,
            title: decl.name.clone(),
            decl,
            host: Some(host),
            spec,
            view: None,
            error: None,
            pending: 1,
            seq_sent: 1,
            seq_shown: 0,
            overrides: HashMap::new(),
            world_label: world.map(|w| format!("{}{}", w.name, if w.writable { "" } else { " (read-only)" })),
            session_origin: origin,
            logs: Vec::new(),
            target: None,
        });
        if focus {
            self.screen = Page::ExtTool(uid);
        }
        wait(uid, 1, rx)
    }

    /// Tasks replaying `OMT_DEBUG_TOOL_EVENTS` into the tool with this uid
    /// (`click:id`, `submit:id:value`, `slot:id:n`, `row:id:n`, `map:x,z`,
    /// plus delayed app actions `reload@secs` — restart the tool from its
    /// files — and `save@secs` — Save the tool's world session).
    fn debug_event_tasks(&mut self, uid: u64) -> Vec<Task<Message>> {
        let origin = self.hub.tool(uid).and_then(|t| t.session_origin.clone());
        let session = origin.and_then(|o| self.sessions.iter().position(|s| s.sess.as_ref().is_some_and(|x| x.origin() == o)));
        std::env::var("OMT_DEBUG_TOOL_EVENTS")
            .unwrap_or_default()
            .split(';')
            .filter_map(|e| {
                if let Some((what, secs)) = e.split_once('@') {
                    let secs: f64 = secs.parse().ok()?;
                    let msg = match what {
                        "reload" => Message::ExtTool(ExtToolMsg::Reload(uid)),
                        "save" => Message::SaveSession(session?),
                        _ => return None,
                    };
                    return Some(Task::perform(blocking(move || std::thread::sleep(std::time::Duration::from_secs_f64(secs))), move |()| msg.clone()));
                }
                let p: Vec<&str> = e.splitn(3, ':').collect();
                let ev = match p[..] {
                    ["click", id] => ToolEvent::Click { id: id.into() },
                    ["submit", id, v] => ToolEvent::Submit { id: id.into(), value: v.into() },
                    ["slot", id, n] => ToolEvent::Slot { id: id.into(), slot: n.parse().ok()? },
                    ["row", id, n] => ToolEvent::Row { id: id.into(), row: n.parse::<usize>().ok()?.saturating_sub(1) },
                    ["map", xz] => {
                        let (x, z) = xz.split_once(',')?;
                        ToolEvent::MapClick { x: x.parse().ok()?, y: None, z: z.parse().ok()?, dim: 0 }
                    }
                    _ => return None,
                };
                Some(self.send_tool(uid, ev))
            })
            .collect()
    }

    fn send_tool(&mut self, uid: u64, ev: ToolEvent) -> Task<Message> {
        let Some(t) = self.hub.tool_mut(uid) else { return Task::none() };
        t.seq_sent += 1;
        t.pending += 1;
        let seq = t.seq_sent;
        let Some(h) = &t.host else {
            t.pending -= 1;
            return Task::none();
        };
        wait(uid, seq, h.send(ev))
    }

    fn tool_output(&mut self, uid: u64, seq: u64, out: ToolOutput) -> Task<Message> {
        let Some(t) = self.hub.tool_mut(uid) else { return Task::none() };
        t.pending = t.pending.saturating_sub(1);
        if seq >= t.seq_shown {
            t.seq_shown = seq;
            if let Some(v) = out.view {
                // Echoed values no longer need the local copy.
                t.overrides.retain(|id, val| input_value(&v, id).is_none_or(|shown| shown != *val));
                t.view = Some(v);
            }
            t.error = out.error;
            if !out.title.is_empty() {
                t.title = out.title;
            }
        }
        t.logs.extend(out.effects.logs);
        let excess = t.logs.len().saturating_sub(200);
        t.logs.drain(..excess);
        let origin = t.session_origin.clone();
        let name = t.title.clone();
        if let (Some(new), Some((tab, path))) = (out.effects.replace, t.target.clone())
            && let Some(et) = self.editor_tabs().into_iter().find(|e| e.uid == tab)
        {
            et.apply_action_edit(&path, new, &name);
            self.status = et.status.clone();
        }
        if let Some(n) = out.effects.notes.last() {
            self.status = format!("{name}: {n}");
        }
        if out.effects.dirty
            && let Some(o) = origin
            && let Some(s) = self.sessions.iter_mut().filter_map(|t| t.sess.as_mut()).find(|s| s.origin() == o)
        {
            s.modified = true;
        }
        match out.effects.clipboard {
            Some(c) => iced::clipboard::write(c),
            None => Task::none(),
        }
    }

    /// The tool tab page.
    pub(crate) fn ext_tool_page(&self, uid: u64) -> Element<'_, Message> {
        let Some(t) = self.hub.tool(uid) else { return text("This tool was closed.").into() };
        let icon = crate::ext_ui::glyph_named(&t.decl.icon).unwrap_or(omt_art::Glyph::Cube);
        let world = t.world_label.clone().unwrap_or_else(|| "no world open — open one in a tool first".into());
        let mut head = row![
            crate::assets::glyph(icon, style::accent(), 24.0),
            column![text(t.title.clone()).size(18).font(style::BOLD), text(format!("{} · {world}", t.decl.ext)).size(11).color(style::muted())].spacing(2),
            space::horizontal(),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if t.pending > 0 {
            let (label, f) = t.host.as_ref().and_then(|h| h.shared().progress()).unwrap_or_else(|| ("working…".into(), -1.0));
            let bar: Element<'_, Message> = if f >= 0.0 { progress_bar(0.0..=1.0, f).length(160).girth(6).into() } else { space::horizontal().width(0).into() };
            head = head.push(column![text(label).size(11).color(style::muted()), bar].spacing(3)).push(button(text("Stop").size(12)).on_press(Message::ExtTool(ExtToolMsg::Stop(uid))).style(style::button_style(Btn::Danger)));
        }
        head = head
            .push(button(text("Reload").size(12)).on_press(Message::ExtTool(ExtToolMsg::Reload(uid))).style(style::button_style(Btn::Standard)))
            .push(button(text("Close").size(12)).on_press(Message::ExtTool(ExtToolMsg::Close(uid))).style(style::button_style(Btn::Ghost)));
        let mut body = column![head].spacing(12);
        if let Some(e) = &t.error {
            body = body.push(container(text(e.clone()).size(12).font(iced::Font::MONOSPACE).color(style::danger())).padding(10).width(Length::Fill).style(style::card()));
        }
        let ctx = crate::ext_ui::Ctx { uid, overrides: &t.overrides, worlds: &self.hub.library, busy: t.pending > 0 };
        match &t.view {
            Some(v) => body = body.push(scrollable(crate::ext_ui::render(v, &ctx)).height(Length::Fill)),
            None => body = body.push(text("starting…").color(style::muted())),
        }
        if !t.logs.is_empty() {
            let tail: Vec<Element<'_, Message>> = t.logs.iter().rev().take(6).rev().map(|(l, m)| text(format!("[{l}] {m}")).size(11).font(iced::Font::MONOSPACE).color(style::hint()).into()).collect();
            body = body.push(container(column(tail).spacing(1)).padding(8).width(Length::Fill).style(style::card_flat()));
        }
        container(body).padding(16).width(Length::Fill).height(Length::Fill).into()
    }
}

impl OmniMCTools {
    /// The Map Viewer's "Extension tools" panel for the world at `origin`:
    /// the open map tool's live UI, or the map tools to open.
    pub(crate) fn map_tools_panel(&self, origin: &std::path::Path) -> Element<'_, Message> {
        if let Some(t) = self.hub.open.iter().find(|t| t.decl.map && t.session_origin.as_deref() == Some(origin)) {
            let uid = t.uid;
            let mut head = row![text(t.title.clone()).size(13).font(style::BOLD), space::horizontal()].spacing(6).align_y(Alignment::Center);
            if t.pending > 0 {
                head = head.push(button(text("Stop").size(11)).padding([2, 8]).on_press(Message::ExtTool(ExtToolMsg::Stop(uid))).style(style::button_style(Btn::Danger)));
            }
            head = head
                .push(button(text("Tab").size(11)).padding([2, 8]).on_press(Message::ExtTool(ExtToolMsg::Open(t.decl.key()))).style(style::button_style(Btn::Ghost)))
                .push(button(text("Close").size(11)).padding([2, 8]).on_press(Message::ExtTool(ExtToolMsg::Close(uid))).style(style::button_style(Btn::Ghost)));
            let mut col = column![head, text("Left-click the map to send a position to the tool.").size(11).color(style::hint())].spacing(8);
            if let Some(e) = &t.error {
                col = col.push(text(e.clone()).size(11).color(style::danger()));
            }
            let ctx = crate::ext_ui::Ctx { uid, overrides: &t.overrides, worlds: &self.hub.library, busy: t.pending > 0 };
            col = col.push(match &t.view {
                Some(v) => crate::ext_ui::render(v, &ctx),
                None => text("starting…").size(12).color(style::muted()).into(),
            });
            return col.into();
        }
        let tools: Vec<&ToolDecl> = self.hub.decls.iter().filter(|d| d.kind == ToolKind::Tool && d.map).collect();
        if tools.is_empty() {
            return text("No map tools installed. Extensions → Install examples adds Chest Explorer.").size(12).color(style::muted()).into();
        }
        column(tools.into_iter().map(|d| {
            container(
                column![
                    text(d.name.clone()).size(13).font(style::BOLD),
                    text(d.description.clone()).size(11).color(style::muted()),
                    button(text("Open here").size(12)).padding([4, 10]).on_press(Message::ExtTool(ExtToolMsg::OpenHere(d.key()))).style(style::button_style(Btn::Primary)),
                ]
                .spacing(4),
            )
            .padding(8)
            .width(Length::Fill)
            .style(style::card())
            .into()
        }))
        .spacing(8)
        .into()
    }
}

/// The value an input shows in a view (`None` = no such input).
fn input_value(v: &UiNode, id: &str) -> Option<String> {
    match v {
        UiNode::Input { id: i, value, .. } if i == id => Some(value.clone()),
        UiNode::Slider { id: i, value, .. } if i == id => Some(value.to_string()),
        UiNode::Column { children, .. } | UiNode::Row { children, .. } | UiNode::Card { children, .. } => children.iter().find_map(|c| input_value(c, id)),
        UiNode::Scroll { child, .. } => input_value(child, id),
        _ => None,
    }
}
