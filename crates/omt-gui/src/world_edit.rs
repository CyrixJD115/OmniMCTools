//! The **World Editor** app (Amulet-style, in Rust): draw a box on the
//! map, set its Y range, then Fill / Replace / Clear / Count / Copy /
//! Paste / export or import structures — applied to the session's
//! workspace copy through `omt-edit`, with an undo history. The map
//! refreshes after each edit. UI in `world_edit_view.rs`.

use std::path::PathBuf;
use std::sync::Arc;

use iced::Task;
use omt_edit::{Block, EditWorld, Matcher, Op, Report, Sel, Undo, Volume};

use crate::mapdata::{MapController, MapMsg};
use crate::mapview::MapEvent;
use crate::session::{Store, WorldSession, blocking};

/// Editor tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Fill,
    Replace,
    Clear,
    Count,
    Clipboard,
    Structure,
    Biome,
    Entities,
}

impl Tool {
    pub const ALL: [Tool; 8] = [Tool::Fill, Tool::Replace, Tool::Clear, Tool::Count, Tool::Clipboard, Tool::Structure, Tool::Biome, Tool::Entities];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Fill => "Fill",
            Tool::Replace => "Replace",
            Tool::Clear => "Clear",
            Tool::Count => "Analyze",
            Tool::Clipboard => "Copy / Paste",
            Tool::Structure => "Structures",
            Tool::Biome => "Biome paint",
            Tool::Entities => "Entities",
        }
    }
}

/// Which text field is being edited (for block suggestions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Fill,
    From,
    To,
}

#[derive(Debug, Clone)]
pub enum EditMsg {
    Tiles(MapMsg),
    Map(MapEvent),
    Tool(Tool),
    Corner(usize, String),
    YMin(i32),
    YMax(i32),
    Block(Field, String),
    Focus(Field),
    Pick(String),
    OnlyAir(bool),
    SkipAir(bool),
    PasteAt(usize, String),
    PasteHere,
    Apply,
    Count,
    Copy,
    Paste,
    Done(String, Result<(Report, Undo), String>),
    Counted(Result<(Arc<Volume>, Report), String>),
    Undo,
    Undone(Result<String, String>),
    Export(bool),
    ExportPicked(bool, Option<PathBuf>),
    Exported(Result<String, String>),
    Import,
    ImportPicked(Option<PathBuf>),
    Imported(Result<Arc<Volume>, String>),
    Dim(u8),
    BiomeName(String),
    ScanEntities,
    Scanned(Result<Vec<(String, u64)>, String>),
    EntityFilter(String),
    RemoveEntities,
    Preview(bool),
    PGrid(Result<Arc<omt_render::blocks3d::BlockGrid>, String>),
    PFrame(iced::widget::image::Handle, Option<iced::widget::image::Allocation>),
    PEv(crate::viewer_3d::Ev3),
    PaintBiome,
    Transform(omt_edit::transform::Xform),
}

pub struct EditorState {
    pub ctl: MapController,
    pub tool: Tool,
    /// Selection corners (block X/Z) + Y range.
    pub a: Option<(i32, i32)>,
    pub b: Option<(i32, i32)>,
    pub y: (i32, i32),
    dragging: bool,
    pub corner_text: [String; 4],
    pub fill: String,
    pub from: String,
    pub to: String,
    pub focus: Field,
    pub only_air: bool,
    pub skip_air: bool,
    pub paste_at: [String; 3],
    pub clipboard: Option<Arc<Volume>>,
    pub counts: Vec<(String, u64)>,
    pub history: Vec<(String, Undo)>,
    pub busy: Option<String>,
    pub status: String,
    pub hover: Option<(f64, f64)>,
    pub biome: String,
    pub entity_counts: Vec<(String, u64)>,
    pub entity_filter: String,
    /// 3D preview of the selection.
    pub preview: crate::world_edit_3d::Preview,
    /// Debug/testing hook (`OMT_DEBUG_EDIT=x1,z1,x2,z2,y1,y2,block`): select
    /// and fill once the map is loaded.
    debug: Option<String>,
}

fn opts(dim: u8) -> omt_render::RenderOptions {
    omt_render::RenderOptions { relief: false, ..omt_render::RenderOptions::for_dimension(dim) }
}

impl EditorState {
    pub fn new() -> Self {
        let mut ctl = MapController::new(0, opts(0));
        ctl.post = Some(omt_render::style::StyleParams::default());
        let (lo, hi) = omt_edit::y_range(0);
        Self {
            ctl,
            tool: Tool::Fill,
            a: None,
            b: None,
            y: (60.max(lo), 80.min(hi)),
            dragging: false,
            corner_text: Default::default(),
            fill: "minecraft:stone".into(),
            from: "minecraft:grass_block".into(),
            to: "minecraft:moss_block".into(),
            focus: Field::Fill,
            only_air: false,
            skip_air: true,
            paste_at: Default::default(),
            clipboard: None,
            counts: Vec::new(),
            history: Vec::new(),
            busy: None,
            status: "Drag on the map to select an area, set its height, choose a tool.".into(),
            hover: None,
            biome: "minecraft:cherry_grove".into(),
            entity_counts: Vec::new(),
            entity_filter: String::new(),
            preview: Default::default(),
            debug: std::env::var("OMT_DEBUG_EDIT").ok(),
        }
    }

    pub fn load(&mut self, sess: &WorldSession) -> Task<EditMsg> {
        self.ctl.load(sess.store.as_ref(), false).map(EditMsg::Tiles)
    }

    /// The current selection box.
    pub fn sel(&self) -> Option<Sel> {
        let (a, b) = (self.a?, self.b?);
        Some(Sel::new([a.0, self.y.0, a.1], [b.0, self.y.1, b.1]))
    }

    fn world(store: Option<&Store>) -> Option<EditWorld> {
        Some(match store? {
            Store::Java(w) => EditWorld::Java(w.clone()),
            Store::Bedrock(w) => EditWorld::Bedrock(w.clone()),
        })
    }

    pub(crate) fn sync_corners(&mut self) {
        self.sync_text();
    }

    fn sync_text(&mut self) {
        if let (Some(a), Some(b)) = (self.a, self.b) {
            self.corner_text = [a.0.to_string(), a.1.to_string(), b.0.to_string(), b.1.to_string()];
        }
    }

    pub fn status_line(&self) -> String {
        match &self.busy {
            Some(b) => format!("{b} · {}", self.ctl.status),
            None => format!("{} · {}", self.status, self.ctl.status),
        }
    }

    fn run(&mut self, label: String, store: Option<&Store>, op: Op) -> Task<EditMsg> {
        let (Some(w), Some(sel)) = (Self::world(store), self.sel()) else {
            self.status = "select an area first".into();
            return Task::none();
        };
        if self.busy.is_some() {
            return Task::none();
        }
        let dim = self.ctl.dim;
        self.busy = Some(format!("{label}…"));
        Task::perform(blocking(move || omt_edit::apply(&w, dim, sel, &op, &|| false).map_err(|e| e.to_string())), move |r| EditMsg::Done(label.clone(), r))
    }

    pub fn update(&mut self, sess: &mut WorldSession, m: EditMsg) -> Task<EditMsg> {
        let store = sess.store.clone();
        let store = store.as_ref();
        match m {
            EditMsg::Tiles(t) => {
                let t = self.ctl.on_msg(t, store).map(EditMsg::Tiles);
                if self.ctl.map.is_some()
                    && let Some(spec) = self.debug.take()
                {
                    let p: Vec<&str> = spec.split(',').collect();
                    let n = |i: usize| p.get(i).and_then(|v| v.trim().parse::<i32>().ok());
                    if let (Some(x1), Some(z1), Some(x2), Some(z2), Some(y1), Some(y2), Some(b)) = (n(0), n(1), n(2), n(3), n(4), n(5), p.get(6)) {
                        self.a = Some((x1, z1));
                        self.b = Some((x2, z2));
                        self.y = (y1, y2);
                        self.fill = b.trim().to_string();
                        self.tool = Tool::Fill;
                        self.sync_text();
                        let apply = self.update(sess, EditMsg::Apply);
                        let preview = if std::env::var_os("OMT_DEBUG_EDIT_PREVIEW").is_some() { self.set_preview(true, store) } else { Task::none() };
                        return Task::batch([t, apply, preview]);
                    }
                }
                return t;
            }
            EditMsg::Map(e) => return self.on_map(e, store),
            EditMsg::Dim(d) => {
                let (lo, hi) = omt_edit::y_range(d);
                self.y = (self.y.0.clamp(lo, hi), self.y.1.clamp(lo, hi));
                return self.ctl.set_dim(d, opts(d), store).map(EditMsg::Tiles);
            }
            EditMsg::Tool(t) => self.tool = t,
            EditMsg::Corner(i, s) => {
                self.corner_text[i] = s;
                let p = |i: usize| self.corner_text[i].trim().parse::<i32>().ok();
                if let (Some(x1), Some(z1), Some(x2), Some(z2)) = (p(0), p(1), p(2), p(3)) {
                    self.a = Some((x1, z1));
                    self.b = Some((x2, z2));
                }
            }
            EditMsg::YMin(v) => self.y = (v.min(self.y.1), self.y.1),
            EditMsg::YMax(v) => self.y = (self.y.0, v.max(self.y.0)),
            EditMsg::Block(f, s) => {
                self.focus = f;
                match f {
                    Field::Fill => self.fill = s,
                    Field::From => self.from = s,
                    Field::To => self.to = s,
                }
            }
            EditMsg::Focus(f) => self.focus = f,
            EditMsg::Pick(name) => match self.focus {
                Field::Fill => self.fill = name,
                Field::From => self.from = name,
                Field::To => self.to = name,
            },
            EditMsg::BiomeName(s) => self.biome = s,
            EditMsg::EntityFilter(s) => self.entity_filter = s,
            EditMsg::ScanEntities => {
                let (Some(w), Some(sel)) = (Self::world(store), self.sel()) else {
                    self.status = "select an area first".into();
                    return Task::none();
                };
                let dim = self.ctl.dim;
                self.busy = Some("Scanning entities…".into());
                return Task::perform(blocking(move || omt_edit::entities::list(&w, dim, sel).map(|l| omt_edit::entities::counts(&l)).map_err(|e| e.to_string())), EditMsg::Scanned);
            }
            EditMsg::Scanned(r) => {
                self.busy = None;
                match r {
                    Ok(c) => {
                        self.status = format!("{} entities in the selection", c.iter().map(|x| x.1).sum::<u64>());
                        self.entity_counts = c;
                    }
                    Err(e) => self.status = format!("scan failed: {e}"),
                }
            }
            EditMsg::RemoveEntities => {
                let (Some(w), Some(sel)) = (Self::world(store), self.sel()) else { return Task::none() };
                if self.busy.is_some() {
                    return Task::none();
                }
                let dim = self.ctl.dim;
                let filter = self.entity_filter.trim().to_string();
                let label = if filter.is_empty() { "Remove all entities".to_string() } else { format!("Remove {filter}") };
                self.busy = Some(format!("{label}…"));
                self.entity_counts.clear();
                return Task::perform(
                    blocking(move || omt_edit::entities::remove(&w, dim, sel, (!filter.is_empty()).then_some(filter.as_str()), &|| false).map_err(|e| e.to_string())),
                    move |r| EditMsg::Done(label.clone(), r),
                );
            }
            EditMsg::Preview(on) => return self.set_preview(on, store),
            EditMsg::PGrid(r) => return self.on_pgrid(r, store),
            EditMsg::PFrame(h, a) => return self.on_pframe(h, a),
            EditMsg::PEv(e) => return self.on_pev(e),
            EditMsg::PaintBiome => {
                let (Some(w), Some(sel)) = (Self::world(store), self.sel()) else {
                    self.status = "select an area first".into();
                    return Task::none();
                };
                if self.busy.is_some() {
                    return Task::none();
                }
                let (dim, biome) = (self.ctl.dim, self.biome.trim().to_string());
                let label = format!("Paint biome {biome}");
                self.busy = Some(format!("{label}…"));
                return Task::perform(blocking(move || omt_edit::biome::paint(&w, dim, sel, &biome, &|| false).map_err(|e| e.to_string())), move |r| EditMsg::Done(label.clone(), r));
            }
            EditMsg::Transform(x) => {
                if let Some(v) = &self.clipboard {
                    let t = omt_edit::transform::transform(v, x);
                    self.status = format!("clipboard {} → {}×{}×{}", match x {
                        omt_edit::transform::Xform::RotateCw => "rotated 90°",
                        omt_edit::transform::Xform::FlipX => "mirrored east–west",
                        omt_edit::transform::Xform::FlipZ => "mirrored north–south",
                    }, t.size[0], t.size[1], t.size[2]);
                    self.clipboard = Some(Arc::new(t));
                }
            }
            EditMsg::OnlyAir(v) => self.only_air = v,
            EditMsg::SkipAir(v) => self.skip_air = v,
            EditMsg::PasteAt(i, s) => self.paste_at[i] = s,
            EditMsg::PasteHere => {
                if let Some(s) = self.sel() {
                    self.paste_at = [s.min[0].to_string(), s.min[1].to_string(), s.min[2].to_string()];
                }
            }
            EditMsg::Apply => {
                let op = match self.tool {
                    Tool::Fill => Op::Fill { to: Block::named(self.fill.trim()), only_air: self.only_air },
                    Tool::Replace => Op::Replace { from: Matcher::parse(&self.from), to: Block::named(self.to.trim()) },
                    Tool::Clear => Op::Clear,
                    _ => return Task::none(),
                };
                let label = match &op {
                    Op::Fill { to, .. } => format!("Fill with {}", to.state.name),
                    Op::Replace { from, to } => format!("Replace {} → {}", from.name, to.state.name),
                    _ => "Clear".into(),
                };
                return self.run(label, store, op);
            }
            EditMsg::Paste => {
                let Some(vol) = self.clipboard.clone() else {
                    self.status = "copy a selection or import a structure first".into();
                    return Task::none();
                };
                let p = |i: usize| self.paste_at[i].trim().parse::<i32>().ok();
                let (Some(x), Some(y), Some(z)) = (p(0), p(1), p(2)) else {
                    self.status = "enter the paste position (X Y Z)".into();
                    return Task::none();
                };
                // The op writes into its own target box; the selection only
                // has to exist for `run`.
                if self.sel().is_none() {
                    self.a = Some((x, z));
                    self.b = Some((x, z));
                }
                return self.run(format!("Paste at {x} {y} {z}"), store, Op::Paste { vol, origin: [x, y, z], skip_air: self.skip_air });
            }
            EditMsg::Count | EditMsg::Copy => {
                let (Some(w), Some(sel)) = (Self::world(store), self.sel()) else {
                    self.status = "select an area first".into();
                    return Task::none();
                };
                let dim = self.ctl.dim;
                self.busy = Some("Reading blocks…".into());
                return Task::perform(blocking(move || omt_edit::read(&w, dim, sel, &|| false).map(|(v, r)| (Arc::new(v), r)).map_err(|e| e.to_string())), EditMsg::Counted);
            }
            EditMsg::Counted(r) => {
                self.busy = None;
                match r {
                    Ok((vol, report)) => {
                        self.status = format!("{} blocks read ({}×{}×{}) — copied to the clipboard", vol.idx.len(), vol.size[0], vol.size[1], vol.size[2]);
                        self.counts = report.counts;
                        self.clipboard = Some(vol);
                        if self.paste_at.iter().all(String::is_empty) {
                            return self.update(sess, EditMsg::PasteHere);
                        }
                    }
                    Err(e) => self.status = format!("read failed: {e}"),
                }
            }
            EditMsg::Done(label, r) => {
                self.busy = None;
                match r {
                    Ok((rep, undo)) => {
                        let extra = if rep.missing_chunks > 0 { format!(" · {} ungenerated chunks skipped", rep.missing_chunks) } else { String::new() };
                        let be = if rep.block_entities_removed > 0 { format!(" · {} block entities removed", rep.block_entities_removed) } else { String::new() };
                        self.status = format!("{label}: {} blocks changed in {} chunks{be}{extra}", rep.changed, rep.chunks);
                        if rep.changed > 0 {
                            self.history.push((label, undo));
                            sess.modified = true;
                            let map = self.ctl.load(store, false).map(EditMsg::Tiles);
                            return Task::batch([map, self.rebuild_preview(store)]);
                        }
                    }
                    Err(e) => self.status = format!("{label} failed: {e}"),
                }
            }
            EditMsg::Undo => {
                let (Some(w), Some((label, undo))) = (Self::world(store), self.history.pop()) else { return Task::none() };
                self.busy = Some("Undoing…".into());
                return Task::perform(blocking(move || omt_edit::undo(&w, &undo).map(|()| format!("undid “{label}”")).map_err(|e| e.to_string())), EditMsg::Undone);
            }
            EditMsg::Undone(r) => {
                self.busy = None;
                self.status = r.unwrap_or_else(|e| format!("undo failed: {e}"));
                let map = self.ctl.load(store, false).map(EditMsg::Tiles);
                return Task::batch([map, self.rebuild_preview(store)]);
            }
            EditMsg::Export(bedrock) => {
                if self.clipboard.is_none() {
                    self.status = "copy a selection first (Copy / Paste → Copy selection)".into();
                    return Task::none();
                }
                let (ext, name) = if bedrock { ("mcstructure", "selection.mcstructure") } else { ("nbt", "selection.nbt") };
                return Task::perform(
                    async move { rfd::AsyncFileDialog::new().set_title("Export structure").add_filter("Structure", &[ext]).set_file_name(name).save_file().await.map(|h| h.path().to_path_buf()) },
                    move |p| EditMsg::ExportPicked(bedrock, p),
                );
            }
            EditMsg::ExportPicked(_, None) | EditMsg::ImportPicked(None) => {}
            EditMsg::ExportPicked(bedrock, Some(path)) => {
                let Some(vol) = self.clipboard.clone() else { return Task::none() };
                let origin = self.sel().map_or([0; 3], |s| s.min);
                let dv = sess.summary.java_data_version.unwrap_or(4671) as i32;
                return Task::perform(
                    blocking(move || {
                        let bytes = if bedrock { omt_edit::structure::to_mcstructure(&vol, origin, None) } else { omt_edit::structure::to_java_structure(&vol, dv) }.map_err(|e| e.to_string())?;
                        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                        Ok(format!("exported to {}", path.display()))
                    }),
                    EditMsg::Exported,
                );
            }
            EditMsg::Exported(r) => self.status = r.unwrap_or_else(|e| format!("export failed: {e}")),
            EditMsg::Import => {
                return Task::perform(
                    async { rfd::AsyncFileDialog::new().set_title("Import a structure").add_filter("Structures", &["mcstructure", "nbt"]).pick_file().await.map(|h| h.path().to_path_buf()) },
                    EditMsg::ImportPicked,
                );
            }
            EditMsg::ImportPicked(Some(path)) => {
                return Task::perform(
                    blocking(move || std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| omt_edit::structure::import(&b).map(Arc::new).map_err(|e| e.to_string()))),
                    EditMsg::Imported,
                );
            }
            EditMsg::Imported(r) => match r {
                Ok(v) => {
                    self.status = format!("structure loaded ({}×{}×{}) — set a position and Paste", v.size[0], v.size[1], v.size[2]);
                    self.clipboard = Some(v);
                    self.tool = Tool::Clipboard;
                }
                Err(e) => self.status = format!("import failed: {e}"),
            },
        }
        Task::none()
    }

    fn on_map(&mut self, e: MapEvent, store: Option<&Store>) -> Task<EditMsg> {
        match e {
            MapEvent::Pan { dx, dz } => {
                self.ctl.camera.cx += dx;
                self.ctl.camera.cz += dz;
                self.ctl.refill(store).map(EditMsg::Tiles)
            }
            MapEvent::Zoom { factor, at } => {
                self.ctl.camera.zoom_at(factor, at);
                self.ctl.refill(store).map(EditMsg::Tiles)
            }
            MapEvent::Resize(s) => self.ctl.on_viewport(s, store).map(EditMsg::Tiles),
            MapEvent::Press { at, right: false, .. } => {
                let p = (at.0.floor() as i32, at.1.floor() as i32);
                self.a = Some(p);
                self.b = Some(p);
                self.dragging = true;
                self.sync_text();
                Task::none()
            }
            MapEvent::Drag { at, .. } if self.dragging => {
                self.b = Some((at.0.floor() as i32, at.1.floor() as i32));
                self.hover = Some(at);
                self.sync_text();
                Task::none()
            }
            MapEvent::Release { at, .. } if self.dragging => {
                self.dragging = false;
                self.b = Some((at.0.floor() as i32, at.1.floor() as i32));
                self.sync_text();
                if let Some(s) = self.sel() {
                    self.status = format!("selected {}×{}×{} = {} blocks", s.size()[0], s.size()[1], s.size()[2], s.volume());
                }
                Task::none()
            }
            MapEvent::Hover { at } | MapEvent::Drag { at, .. } => {
                self.hover = Some(at);
                Task::none()
            }
            MapEvent::Press { .. } | MapEvent::Release { .. } => Task::none(),
        }
    }
}
