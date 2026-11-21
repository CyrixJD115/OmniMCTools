//! NBT Editor world session: UMT's editor with its side tabs (World
//! Settings · Chunk Locator · LevelDB Search), a world tree of files and
//! players, and multi-document NBT tabs whose saves go back into the
//! workspace (region chunks, LevelDB keys, files) — the original world is
//! only replaced on the session's Save.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Element, Length, Task};
use omt_nbt::{ByteOrder, Nbt, NbtDocument, Tag};

use crate::editor::{EditorMsg, EditorTab, Sink};
use crate::ldb_view::{LdbMsg, LdbView};
use crate::locator::{LocMsg, Locator, OpenWhat};
use crate::session::{Store, WorldSession};
use crate::style::{self, Btn};
use crate::world_settings::{WorldSettings, WsMsg};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    WorldSettings,
    Locator,
    LevelDb,
    Doc(usize),
}

#[derive(Debug, Clone)]
enum Entry {
    File(PathBuf),
    Key(Vec<u8>),
}

#[derive(Debug, Clone)]
pub enum NbtMsg {
    Page(Page),
    OpenEntry(usize),
    CloseDoc(usize),
    Doc(usize, EditorMsg),
    Ws(WsMsg),
    Loc(LocMsg),
    Ldb(LdbMsg),
}

pub struct NbtSession {
    page: Page,
    docs: Vec<EditorTab>,
    /// Shared extension manager, handed to every opened document.
    ext: Option<std::sync::Arc<omt_script::Manager>>,
    entries: Vec<(String, Entry)>,
    ws: WorldSettings,
    loc: Locator,
    ldb: LdbView,
    pub status: String,
}

/// Offline player-name resolution (UMT resolves via the Mojang API; OMT
/// stays offline and reads the launcher's `usercache.json` when present).
fn usercache(origin: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for dir in origin.ancestors().skip(1).take(3) {
        let Ok(text) = std::fs::read_to_string(dir.join("usercache.json")) else { continue };
        if let Ok(serde_json::Value::Array(items)) = serde_json::from_str(&text) {
            for it in items {
                if let (Some(n), Some(u)) = (it["name"].as_str(), it["uuid"].as_str()) {
                    out.insert(u.to_lowercase(), n.to_string());
                }
            }
        }
    }
    out
}

fn list_dat(dir: &Path, prefix: &str, out: &mut Vec<(String, Entry)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "dat")).collect();
    v.sort();
    for p in v {
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        out.push((format!("{prefix}{name}"), Entry::File(p)));
    }
}

impl NbtSession {
    pub fn new(sess: &WorldSession, nether_default: i32, ext: std::sync::Arc<omt_script::Manager>) -> Self {
        let root = sess.root();
        let bedrock = sess.edition() == omt_world::Edition::Bedrock;
        let mut entries = vec![("📄 level.dat".to_string(), Entry::File(root.join("level.dat")))];
        match &sess.store {
            Some(Store::Java(w)) => {
                let names = usercache(&sess.origin());
                let pdir = w.players_dir();
                for uuid in w.list_players().unwrap_or_default() {
                    let label = match names.get(&uuid.to_lowercase()) {
                        Some(n) => format!("👤 {n}"),
                        None => format!("👤 {uuid}"),
                    };
                    entries.push((label, Entry::File(pdir.join(format!("{uuid}.dat")))));
                }
                list_dat(&root.join("data/minecraft"), "🗄 data/", &mut entries);
                list_dat(&root.join("data"), "🗄 data/", &mut entries);
                for dim in ["overworld", "the_nether", "the_end"] {
                    list_dat(&root.join(format!("dimensions/minecraft/{dim}/data/minecraft")), &format!("🗄 {dim}/"), &mut entries);
                }
            }
            Some(Store::Bedrock(w)) => {
                let keys = w.key_index().unwrap_or_default();
                let mut named: Vec<(String, Vec<u8>)> = keys
                    .into_iter()
                    .filter_map(|(k, _)| match omt_bedrock::keys::decode_key(&k) {
                        omt_bedrock::keys::DbKey::LocalPlayer => Some(("👤 Local Player".to_string(), k)),
                        omt_bedrock::keys::DbKey::Player { .. } => Some((format!("👤 {}", String::from_utf8_lossy(&k)), k)),
                        omt_bedrock::keys::DbKey::Other if k.iter().all(|b| b.is_ascii_graphic()) => {
                            let s = String::from_utf8_lossy(&k).into_owned();
                            let icon = if s.starts_with("player") { "👤" } else if s.starts_with("map_") { "🗺" } else { "🗄" };
                            Some((format!("{icon} {s}"), k))
                        }
                        _ => None,
                    })
                    .collect();
                named.sort_by(|a, b| a.0.cmp(&b.0));
                entries.extend(named.into_iter().map(|(l, k)| (l, Entry::Key(k))));
            }
            None => {}
        }
        Self {
            page: Page::WorldSettings,
            docs: Vec::new(),
            ext: Some(ext),
            entries,
            ws: WorldSettings::load(&root, bedrock),
            loc: Locator::new(nether_default),
            ldb: LdbView::default(),
            status: String::new(),
        }
    }

    pub fn dirty_docs(&self) -> usize {
        self.docs.iter().filter(|d| d.is_dirty()).count()
    }

    /// The open documents (extension actions reach their tabs).
    pub(crate) fn docs_mut(&mut self) -> &mut Vec<EditorTab> {
        &mut self.docs
    }

    fn push_doc(&mut self, mut tab: EditorTab) {
        if let Some(mgr) = self.ext.clone() {
            tab.set_extensions(mgr);
        }
        self.docs.push(tab);
        self.page = Page::Doc(self.docs.len() - 1);
    }

    fn open_key(&mut self, store: &Store, key: &[u8], title: String) {
        let Store::Bedrock(w) = store else { return };
        let Ok(Some(v)) = w.raw_get(key) else {
            self.status = "key not found".into();
            return;
        };
        match decode_le(&v) {
            Some((nbt, single, hints, entry_hints)) => {
                let sink = if single { Sink::BedrockKey(key.to_vec()) } else { Sink::BedrockSequence(key.to_vec()) };
                let mut tab = EditorTab::from_nbt(title, nbt, ByteOrder::Little, sink);
                if !single {
                    tab.entry_hints = entry_hints;
                } else {
                    tab.doc.list_hints = hints;
                }
                self.push_doc(tab);
            }
            None => self.status = "this value is not NBT — see the hex view in LevelDB Search".into(),
        }
    }

    fn open_chunk(&mut self, store: &Store, what: OpenWhat, dim: u8, x: i32, z: i32) {
        match (store, what) {
            (Store::Java(w), OpenWhat::Chunk | OpenWhat::Entities | OpenWhat::BlockEntities) => {
                let kind = if what == OpenWhat::Entities { "entities" } else { "region" };
                let Some(dir) = w.dimension_subdir(dim, kind) else { return };
                let region = dir.join(format!("r.{}.{}.mca", x.div_euclid(32), z.div_euclid(32)));
                let loaded = std::fs::read(&region).ok().and_then(|bytes| {
                    let r = omt_java::RegionFile::from_bytes(&region, &bytes).ok()?;
                    r.read_chunk(&bytes, x, z).ok()
                });
                match loaded {
                    Some(nbt) => {
                        let title = format!("{} {x}:{z}", if kind == "entities" { "Entities" } else { "Chunk" });
                        // Re-read the raw payload to capture empty-list types.
                        let mut tab = EditorTab::from_nbt(title, nbt, ByteOrder::Big, Sink::JavaChunk { region: region.clone(), x, z });
                        if let Ok(bytes) = std::fs::read(&region)
                            && let Ok(r) = omt_java::RegionFile::from_bytes(&region, &bytes)
                            && let Ok(raw) = r.read_raw(&bytes, x, z)
                            && let Ok((_, _, hints)) = omt_nbt::read_prefix_with_hints(&raw, ByteOrder::Big)
                        {
                            tab.doc.list_hints = hints;
                        }
                        self.push_doc(tab);
                    }
                    None => self.status = format!("no {kind} data for chunk {x}:{z}"),
                }
            }
            (Store::Bedrock(w), OpenWhat::Entities) => {
                let ids = w.raw_get(&omt_bedrock::keys::digp_key(x, z, dim)).ok().flatten().unwrap_or_default();
                let mut keys = Vec::new();
                let mut entries = Vec::new();
                let mut actor_hints = Vec::new();
                for id in ids.as_chunks::<8>().0 {
                    let k = omt_bedrock::keys::actor_key(*id);
                    if let Ok(Some(v)) = w.raw_get(&k)
                        && let Ok((n, _, hints)) = omt_nbt::read_prefix_with_hints(&v, ByteOrder::Little)
                    {
                        keys.push(k);
                        entries.push(n.tag);
                        actor_hints.push(hints);
                    }
                }
                if entries.is_empty() {
                    self.status = format!("no entities in chunk {x}:{z}");
                    return;
                }
                let nbt = Nbt { name: String::new(), tag: Tag::Compound(vec![("entries".into(), Tag::List(entries))]) };
                let mut tab = EditorTab::from_nbt(format!("Entities {x}:{z}"), nbt, ByteOrder::Little, Sink::BedrockActors(keys));
                tab.entry_hints = actor_hints;
                self.push_doc(tab);
            }
            (Store::Bedrock(_), _) => {
                let key = omt_bedrock::keys::chunk_record(x, z, dim, omt_bedrock::keys::tag::BLOCK_ENTITIES);
                self.open_key(store, &key, format!("Block Entities {x}:{z}"));
            }
        }
    }

    /// Write a document's pending save into the workspace store.
    fn write_pending(&mut self, store: &Store, i: usize) -> bool {
        let Some(tab) = self.docs.get(i) else { return false };
        if !tab.pending_save {
            return false;
        }
        let result: Result<(), String> = (|| {
            let bytes = tab.sink_bytes()?;
            match (&tab.sink, store) {
                (Sink::WorkspaceFile(p), _) => {
                    let tmp = p.with_extension("omt_tmp");
                    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
                    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
                }
                (Sink::JavaChunk { region, x, z }, _) => {
                    let old = std::fs::read(region).unwrap_or_default();
                    let r = if old.len() >= omt_java::region::HEADER_BYTES {
                        omt_java::RegionFile::from_bytes(region, &old).map_err(|e| e.to_string())?
                    } else {
                        let empty = vec![0u8; omt_java::region::HEADER_BYTES];
                        omt_java::RegionFile::from_bytes(region, &empty).map_err(|e| e.to_string())?
                    };
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
                    let image = r.with_chunk(&old, *x, *z, &bytes, now);
                    let tmp = region.with_extension("mca.omt_tmp");
                    std::fs::write(&tmp, image).map_err(|e| e.to_string())?;
                    std::fs::rename(&tmp, region).map_err(|e| e.to_string())
                }
                (Sink::BedrockKey(k) | Sink::BedrockSequence(k), Store::Bedrock(w)) => {
                    w.raw_put_many(&[(k.clone(), bytes)]).map_err(|e| e.to_string())
                }
                (Sink::BedrockActors(keys), Store::Bedrock(w)) => {
                    let entries = tab.doc.root().get("entries").and_then(|t| t.as_list()).map(<[Tag]>::to_vec).unwrap_or_default();
                    if entries.len() != keys.len() {
                        return Err("add/remove of whole entities is not supported here — edit their fields".into());
                    }
                    let mut kvs = Vec::new();
                    for (k, e) in keys.iter().zip(entries) {
                        let n = Nbt { name: String::new(), tag: e };
                        kvs.push((k.clone(), omt_nbt::write(&n, ByteOrder::Little).map_err(|e| e.to_string())?));
                    }
                    w.raw_put_many(&kvs).map_err(|e| e.to_string())
                }
                _ => Err("this document cannot be saved into this world".into()),
            }
        })();
        let tab = &mut self.docs[i];
        match result {
            Ok(()) => {
                tab.mark_saved("saved into the workspace — Save the world to keep it".into());
                true
            }
            Err(e) => {
                tab.pending_save = false;
                tab.status = format!("save FAILED: {e}");
                false
            }
        }
    }

    pub fn update(&mut self, sess: &mut WorldSession, msg: NbtMsg) -> Task<NbtMsg> {
        let store = sess.store.clone();
        match msg {
            NbtMsg::Page(p) => {
                self.page = p;
                if p == Page::Locator && !self.loc.is_loaded() {
                    return self.loc.load(store).map(NbtMsg::Loc);
                }
                if p == Page::LevelDb && !self.ldb.is_loaded() {
                    return self.ldb.load(store).map(NbtMsg::Ldb);
                }
            }
            NbtMsg::OpenEntry(i) => {
                let Some((label, entry)) = self.entries.get(i).cloned() else { return Task::none() };
                match entry {
                    Entry::File(p) => match NbtDocument::load(&p) {
                        Ok(doc) => {
                            let title = label.split_once(' ').map(|x| x.1.to_string()).unwrap_or(label);
                            self.push_doc(EditorTab::with_doc(doc, title, Sink::WorkspaceFile(p)));
                        }
                        Err(e) => self.status = format!("open failed: {e}"),
                    },
                    Entry::Key(k) => {
                        if let Some(s) = &store {
                            self.open_key(s, &k, label.split_once(' ').map(|x| x.1.to_string()).unwrap_or(label));
                        }
                    }
                }
            }
            NbtMsg::CloseDoc(i) => {
                if i < self.docs.len() {
                    if self.docs[i].is_dirty() && !self.docs[i].status.starts_with("unsaved") {
                        self.docs[i].status = "unsaved changes — press ✕ again to discard them".into();
                        return Task::none();
                    }
                    self.docs.remove(i);
                    self.page = if self.docs.is_empty() { Page::WorldSettings } else { Page::Doc(i.min(self.docs.len() - 1)) };
                }
            }
            NbtMsg::Doc(i, m) => {
                if let Some(tab) = self.docs.get_mut(i) {
                    let tasks = tab.update(m);
                    if tab.pending_save
                        && let Some(s) = &store
                        && self.write_pending(s, i)
                    {
                        sess.modified = true;
                    }
                    return Task::batch(tasks.into_iter().map(move |t| t.map(move |m| NbtMsg::Doc(i, m))));
                }
            }
            NbtMsg::Ws(m) => {
                if self.ws.update(m) {
                    sess.modified = true;
                }
            }
            NbtMsg::Loc(m) => {
                let (task, open) = self.loc.update(m, store.clone());
                if let (Some((what, dim, x, z)), Some(s)) = (open, &store) {
                    self.open_chunk(s, what, dim, x, z);
                }
                return task.map(NbtMsg::Loc);
            }
            NbtMsg::Ldb(m) => {
                let (task, open) = self.ldb.update(m, store.clone());
                if let (Some(k), Some(s)) = (open, &store) {
                    self.open_key(s, &k, crate::ldb_view::describe(&k));
                }
                return task.map(NbtMsg::Ldb);
            }
        }
        Task::none()
    }

    /// Route a global hotkey to the active document.
    pub fn active_doc(&self) -> Option<usize> {
        match self.page {
            Page::Doc(i) if i < self.docs.len() => Some(i),
            _ => None,
        }
    }

    pub fn view<'a>(&'a self, sess: &'a WorldSession) -> Element<'a, NbtMsg> {
        let bedrock = sess.edition() == omt_world::Edition::Bedrock;
        let nav = |label: &'static str, p: Page| {
            button(text(label).size(13))
                .on_press(NbtMsg::Page(p))
                .width(Length::Fill)
                .style(style::button_style(if self.page == p { Btn::Primary } else { Btn::Ghost }))
        };
        let mut side = column![nav("⚙  World Settings", Page::WorldSettings), nav("🗺  Chunk Locator", Page::Locator)].spacing(2);
        if bedrock {
            side = side.push(nav("🔎  LevelDB Search", Page::LevelDb));
        }
        if !self.docs.is_empty() {
            side = side.push(text("Open documents").size(12).color(style::muted()));
            for (i, d) in self.docs.iter().enumerate() {
                let dot = if d.is_dirty() { " ●" } else { "" };
                side = side.push(
                    row![
                        button(text(format!("{}{dot}", d.title)).size(12))
                            .on_press(NbtMsg::Page(Page::Doc(i)))
                            .width(Length::Fill)
                            .style(style::button_style(if self.page == Page::Doc(i) { Btn::Primary } else { Btn::Ghost })),
                        button(text("✕").size(10)).on_press(NbtMsg::CloseDoc(i)).style(style::button_style(Btn::Ghost)),
                    ]
                    .align_y(iced::Alignment::Center),
                );
            }
        }
        let files = self.entries.iter().enumerate().fold(column![].spacing(1), |c, (i, (label, _))| {
            c.push(button(text(label.clone()).size(12)).on_press(NbtMsg::OpenEntry(i)).width(Length::Fill).style(style::button_style(Btn::Ghost)))
        });
        let sidebar = column![
            side,
            iced::widget::rule::horizontal(1.0),
            text(format!("World files ({})", self.entries.len())).size(12).color(style::muted()),
            scrollable(files).height(Length::Fill),
            text(self.status.clone()).size(11).color(style::warn()),
        ]
        .spacing(6)
        .padding(8)
        .width(250);
        let main: Element<'a, NbtMsg> = match self.page {
            Page::WorldSettings => self.ws.view().map(NbtMsg::Ws),
            Page::Locator => self.loc.view(bedrock).map(NbtMsg::Loc),
            Page::LevelDb => self.ldb.view().map(NbtMsg::Ldb),
            Page::Doc(i) => match self.docs.get(i) {
                Some(tab) => container(crate::editor_view::view(tab, std::rc::Rc::new(move |m| NbtMsg::Doc(i, m)))).padding(8).into(),
                None => space::vertical().into(),
            },
        };
        row![container(sidebar).height(Length::Fill).style(style::panel()), container(main).width(Length::Fill).height(Length::Fill)].into()
    }
}

/// Parse a Bedrock value as one LE root (`true`) or a sequence of roots
/// wrapped as `{entries: [...]}` (`false`).
/// Decode one value into an editor document, capturing empty-list element
/// types (root hints for single roots, per-entry hints for sequences) so
/// saving without edits reproduces the original bytes.
fn decode_le(v: &[u8]) -> Option<(Nbt, bool, omt_nbt::ListHints, Vec<omt_nbt::ListHints>)> {
    if let Ok((n, _, hints)) = omt_nbt::read_prefix_with_hints(v, ByteOrder::Little) {
        return Some((n, true, hints, Vec::new()));
    }
    let mut entries = Vec::new();
    let mut entry_hints = Vec::new();
    let mut pos = 0;
    while pos < v.len() {
        let (n, used, hints) = omt_nbt::read_prefix_with_hints(&v[pos..], ByteOrder::Little).ok()?;
        if used == 0 {
            return None;
        }
        entries.push(n.tag);
        entry_hints.push(hints);
        pos += used;
    }
    if entries.is_empty() {
        return Some((Nbt { name: String::new(), tag: Tag::Compound(vec![("entries".into(), Tag::List(vec![]))]) }, false, Default::default(), Vec::new()));
    }
    Some((Nbt { name: String::new(), tag: Tag::Compound(vec![("entries".into(), Tag::List(entries))]) }, false, Default::default(), entry_hints))
}
