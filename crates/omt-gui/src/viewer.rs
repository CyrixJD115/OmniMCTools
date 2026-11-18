//! The standalone **Map Viewer** app: a world opened in place, read-only,
//! on the shared LOD tile engine (`mapdata`) with viewer-only features —
//! map styles and sun lighting applied as an instant post-process
//! (`omt_render::style`), underground (cave) mode, slicer, chunk/region
//! grids, slime chunks, spawn/player/bookmark pins, a scale bar + compass,
//! go-to and PNG export of the current view. The UI lives in
//! `viewer_view.rs`.

use std::path::PathBuf;
use std::sync::Arc;

use iced::Task;
use omt_render::markers::PlayerMarker;
use omt_render::style::MapStyle;
use omt_render::RenderOptions;

use crate::mapdata::{MapController, MapMsg};
use crate::mapview::MapEvent;
use crate::session::{Store, WorldSession, blocking};

pub use crate::viewer_types::{Bookmark, Layer, PanelTab, ViewerKey, ViewerPrefs};

#[derive(Debug, Clone)]
pub enum ViewerMsg {
    Tiles(MapMsg),
    Map(MapEvent),
    Dim(u8),
    Style(MapStyle),
    Azimuth(f32),
    Altitude(f32),
    Strength(f32),
    Contour(i32),
    Ambient(f32),
    Shadows(f32),
    ChunkGrid(Option<bool>),
    Layer(Layer, bool),
    Cave(bool),
    Slicer(bool),
    SlicerY(i32),
    /// Slicer floor: hide blocks below Y (`None` = off).
    SlicerMin(Option<i32>),
    GotoX(String),
    GotoZ(String),
    Goto,
    Fit,
    Zoom(f32),
    BookmarkName(String),
    AddBookmark,
    GoBookmark(usize),
    RemoveBookmark(usize),
    Export,
    ExportPicked(Option<PathBuf>),
    Exported(Result<String, String>),
    Markers(Arc<(Vec<PlayerMarker>, Option<i64>)>),
    MapUi(crate::mapdata::MapUiMsg),
    TexCell((i32, i32), u64, Option<iced::widget::image::Handle>),
    /// 3D terrain view on/off.
    Mode3D(bool),
    /// Isometric view at a view angle (0..3); `None` = 2D map.
    Iso(Option<u8>),
    /// Iso detail: flat map colors or textured block looks.
    IsoTextured(bool),
    Ev3(crate::viewer_3d::Ev3),
    Grid3(Arc<omt_render::voxel::Grid>, Option<crate::viewer_3d::GpuMesh>),
    Gpu3(bool),
    /// A rendered 3D frame, already uploaded (drawn without a blank frame).
    Frame3(iced::widget::image::Handle, Option<iced::widget::image::Allocation>),
    /// The window's scale factor (physical pixels per logical pixel).
    Dpi3(f32),
    /// 3D render resolution (fraction of native).
    Quality3(f32),
    /// 3D block render distance.
    BlockDist3(i32),
    Controls3(crate::viewer_3d_input::Controls3),
    Move3(crate::viewer_3d_input::Move3),
    LookSens3(f32),
    FlySpeed3(f32),
    Far3(f32),
    Scale3(f32),
    /// 3D detail: `false` terrain (height field), `true` blocks.
    Detail3(bool),
    Ruler(crate::mapview_overlay::Ruler),
    Entities((i32, i32, i32, i32), Arc<Vec<omt_render::markers::EntityMarker>>),
    BGrid(Result<(Arc<omt_render::blocks3d::BlockGrid>, Option<crate::viewer_3d::GpuBlocks>), String>),
    /// Open this world in another tool (the compact "Tools" menu).
    OpenIn(crate::session::App),
    /// Center on the world spawn (switches to the Overworld first).
    GotoSpawn,
    /// Center on the nearest player marker.
    GotoPlayer,
    /// Copy the pointed block as a `/tp x y z` command.
    CopyTp,
    /// Back to 1:1 zoom (keyboard `*`).
    ZoomReset,
    /// Auto → on → off → auto (keyboard Ctrl+G).
    CycleChunkGrid,
    /// Open/close a side-panel tab.
    Panel(Option<PanelTab>),
    /// The floating "Tools" menu is open or closed.
    ToolsMenu(bool),
    /// Keyboard navigation from the app subscription.
    Key(ViewerKey),
    /// uNmINeD-parity panels (see `viewer_ext`).
    Ext(crate::viewer_ext::ExtMsg),
}

pub struct ViewerState {
    pub ctl: MapController,
    pub prefs: ViewerPrefs,
    pub cave: bool,
    pub slicer: Option<i32>,
    /// Slicer floor (hide below).
    pub slicer_min: Option<i32>,
    pub goto: (String, String),
    pub bookmark_name: String,
    pub players: Vec<PlayerMarker>,
    pub seed: Option<i64>,
    pub hover: Option<(f64, f64)>,
    pub status: String,
    /// Slime chunks of the last planned view (computed off the draw path).
    pub slime: Vec<(i32, i32)>,
    slime_rect: Option<(i32, i32, i32, i32)>,
    pub origin: PathBuf,
    pub bedrock: bool,
    /// Block-texture layer (zoomed-in view).
    pub tex: crate::viewer_tex::TexLayer,
    /// 3D terrain view.
    pub v3: crate::viewer_3d::View3,
    /// Loaded tiles were re-lit: the 3D grid must be rebuilt.
    regrid: bool,
    /// Debug/testing hook (`OMT_DEBUG_VIEWER_3D=1|blocks[@x,z]`): enter
    /// 3D (optionally block detail at a position) once loaded.
    debug3d: Option<String>,
    /// Debug/testing hook (`OMT_DEBUG_VIEWER_MENU=1`): keep the Tools menu
    /// pinned open (for headless screenshots).
    debug_menu: bool,
    /// Biome legend of the loaded tiles (Biomes style) + tile count it saw.
    pub legend: Vec<(String, usize)>,
    legend_tiles: usize,
    /// Entities of the last queried visible chunk rect.
    pub entities: Arc<Vec<omt_render::markers::EntityMarker>>,
    ent_rect: Option<(i32, i32, i32, i32)>,
    ent_busy: bool,
    /// Distance measurement (left-drag on the map).
    pub measure: Option<[(f64, f64); 2]>,
    measuring: bool,
    /// The floating "Tools" menu (open world in another tool) is showing.
    pub tools_menu: bool,
    /// Extension panels' state (`viewer_ext`).
    pub ext: crate::viewer_ext::Ext,
}

impl ViewerState {
    pub fn new(sess: &WorldSession, mut prefs: ViewerPrefs) -> Self {
        if let Some(d) = prefs.default_preset.clone()
            && let Some(p) = prefs.presets.iter().find(|p| p.name == d).cloned()
        {
            p.apply_to(&mut prefs);
        }
        let rules = omt_render::rules::register(prefs.rules.compile());
        let mut ctl = MapController::new(0, Self::opts_for(0, false, None, prefs.biome_tint, rules));
        ctl.post = Some(prefs.style);
        Self {
            ctl,
            prefs,
            cave: false,
            slicer: None,
            slicer_min: None,
            goto: (String::new(), String::new()),
            bookmark_name: String::new(),
            players: Vec::new(),
            seed: None,
            hover: None,
            status: String::new(),
            slime: Vec::new(),
            slime_rect: None,
            origin: sess.origin(),
            bedrock: sess.edition() == omt_world::Edition::Bedrock,
            tex: Default::default(),
            v3: crate::viewer_3d::View3 { gpu: true, ..Default::default() },
            regrid: false,
            debug3d: std::env::var("OMT_DEBUG_VIEWER_3D").ok(),
            debug_menu: std::env::var("OMT_DEBUG_VIEWER_MENU").is_ok(),
            legend: Vec::new(),
            legend_tiles: usize::MAX,
            entities: Arc::default(),
            ent_rect: None,
            ent_busy: false,
            measure: None,
            measuring: false,
            // `OMT_DEBUG_VIEWER_MENU=1` pins the Tools menu open (screenshots).
            tools_menu: std::env::var("OMT_DEBUG_VIEWER_MENU").is_ok(),
            ext: crate::viewer_ext::Ext::with_rules(rules),
        }
    }

    /// Render options: no baked relief (the style lights the map).
    fn opts_for(dim: u8, cave: bool, slicer: Option<i32>, tint: bool, rules: u64) -> RenderOptions {
        let base = RenderOptions::for_dimension(dim);
        RenderOptions { relief: false, cave, max_y: slicer.or(base.max_y), biome_tint: tint, rules, ..base }
    }

    /// Render options for dimension `dim` with the current look.
    fn opts_dim(&self, dim: u8) -> RenderOptions {
        let mut o = Self::opts_for(dim, self.cave, self.slicer, self.prefs.biome_tint, self.ext.rules_id);
        if let Some(m) = self.slicer_min {
            o.min_y = o.min_y.max(m);
        }
        o
    }

    fn opts(&self) -> RenderOptions {
        self.opts_dim(self.ctl.dim)
    }

    /// Switch to `dim` (when needed) and center on (x, z).
    pub(crate) fn goto_dim(&mut self, sess: &WorldSession, dim: u8, x: f64, z: f64) -> Task<ViewerMsg> {
        let store = sess.store.as_ref();
        let mut tasks = Vec::new();
        if dim != self.ctl.dim {
            let o = self.opts_dim(dim);
            tasks.push(self.ctl.set_dim(dim, o, store).map(ViewerMsg::Tiles));
        }
        self.center(x, z);
        tasks.push(self.ctl.refill(store).map(ViewerMsg::Tiles));
        Task::batch(tasks)
    }

    /// Start: load the map and read markers (players, seed) in the background.
    pub fn load(&mut self, sess: &WorldSession) -> Task<ViewerMsg> {
        let tiles = self.ctl.load(sess.store.as_ref(), false).map(ViewerMsg::Tiles);
        let store = sess.store.clone();
        let root = sess.root();
        let markers = Task::perform(
            blocking(move || match store {
                Some(Store::Java(w)) => (omt_render::markers::java_players(&w), omt_render::markers::java_seed(&root)),
                Some(Store::Bedrock(w)) => {
                    let seed = w.level_dat.nbt.tag.get("RandomSeed").and_then(omt_nbt::Tag::as_i64);
                    (omt_render::markers::bedrock_players(&w), seed)
                }
                None => (Vec::new(), None),
            }),
            |m| ViewerMsg::Markers(Arc::new(m)),
        );
        let ext = self.ext_load(sess);
        Task::batch([tiles, markers, ext])
    }

    pub fn status_line(&self) -> String {
        match (self.status.is_empty(), self.ctl.status.is_empty()) {
            (true, _) => self.ctl.status.clone(),
            (false, true) => self.status.clone(),
            (false, false) => format!("{} — {}", self.status, self.ctl.status),
        }
    }

    pub(crate) fn rerender(&mut self, store: Option<&Store>) -> Task<ViewerMsg> {
        let o = self.opts();
        self.ctl.set_options(o, store).map(ViewerMsg::Tiles)
    }

    pub(crate) fn restyle(&mut self) {
        self.ctl.set_post(Some(self.prefs.style));
        self.regrid = true;
    }

    /// Recompute the slime chunk list when the visible chunk rect changed
    /// (bounded: nothing when zoomed far out).
    fn plan_slime(&mut self) {
        if !self.prefs.slime || self.ctl.dim != 0 || (!self.bedrock && self.seed.is_none()) {
            self.slime.clear();
            self.slime_rect = None;
            return;
        }
        let (x0, z0, x1, z1) = self.ctl.camera.visible_blocks(self.ctl.view);
        let r = ((x0 / 16.0).floor() as i32, (z0 / 16.0).floor() as i32, (x1 / 16.0).ceil() as i32, (z1 / 16.0).ceil() as i32);
        if self.slime_rect == Some(r) {
            return;
        }
        self.slime_rect = Some(r);
        self.slime.clear();
        let area = (r.2 - r.0 + 1) as i64 * (r.3 - r.1 + 1) as i64;
        if area > 60_000 {
            return;
        }
        let (bedrock, seed) = (self.bedrock, self.seed.unwrap_or(0));
        let Some(map) = self.ctl.map.as_ref() else { return };
        for cz in r.1..=r.3 {
            for cx in r.0..=r.2 {
                // Only where the world has chunks (no checkerboard over the void).
                if !map.chunk_present(cx, cz) {
                    continue;
                }
                let slime = if bedrock { omt_render::markers::bedrock_slime(cx, cz) } else { omt_render::markers::java_slime(seed, cx, cz) };
                if slime {
                    self.slime.push((cx, cz));
                }
            }
        }
    }

    pub fn update(&mut self, sess: &WorldSession, bookmarks: &mut Vec<Bookmark>, m: ViewerMsg) -> Task<ViewerMsg> {
        // Any interaction closes the floating "Tools" menu; background tile
        // work, viewport resizes and panel toggles don't. The debug hook
        // (`OMT_DEBUG_VIEWER_MENU=1`) pins it open for screenshots.
        if !self.debug_menu {
            match &m {
                ViewerMsg::Tiles(_)
                | ViewerMsg::TexCell(_, _, _)
                | ViewerMsg::Markers(_)
                | ViewerMsg::Entities(_, _)
                | ViewerMsg::Frame3(_, _)
                | ViewerMsg::Grid3(_, _)
                | ViewerMsg::BGrid(_)
                | ViewerMsg::Dpi3(_)
                | ViewerMsg::Ev3(_)
                | ViewerMsg::ToolsMenu(_)
                | ViewerMsg::Panel(_)
                | ViewerMsg::Ext(_) => {}
                ViewerMsg::Map(MapEvent::Resize(_)) => {}
                _ => self.tools_menu = false,
            }
        }
        let store = sess.store.as_ref();
        let task = match m {
            ViewerMsg::Tiles(t) => {
                let t = self.ctl.on_msg(t, store).map(ViewerMsg::Tiles);
                self.regrid |= self.v3.on;
                if self.ctl.map.is_some()
                    && let Some(spec) = self.debug3d.take()
                {
                    let (mode, at) = spec.split_once('@').unwrap_or((spec.as_str(), ""));
                    let xz: Vec<f64> = at.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                    if xz.len() == 2 {
                        self.ctl.camera.cx = xz[0];
                        self.ctl.camera.cz = xz[1];
                    }
                    if let Some(rot) = mode.strip_prefix("isot") {
                        // Isometric + textured detail (headless testing).
                        self.v3.iso_textured = true;
                        return Task::batch([t, self.set_iso(Some(rot.parse().unwrap_or(0)), store)]);
                    }
                    if let Some(rot) = mode.strip_prefix("iso") {
                        return Task::batch([t, self.set_iso(Some(rot.parse().unwrap_or(0)), store)]);
                    }
                    let enter = self.set_3d(true, store);
                    let detail = if mode == "blocks" { self.set_detail(true, store) } else { Task::none() };
                    return Task::batch([t, enter, detail]);
                }
                t
            }
            ViewerMsg::Mode3D(on) if on && self.v3.iso.is_some() => {
                // Iso → perspective 3D: same grid, perspective camera.
                self.v3.iso = None;
                self.request_grid()
            }
            ViewerMsg::Mode3D(on) => self.set_3d(on, store),
            ViewerMsg::Iso(rot) => self.set_iso(rot, store),
            ViewerMsg::IsoTextured(on) => {
                if self.v3.iso_textured != on {
                    self.v3.iso_textured = on;
                    self.v3.iso_pan = (0.0, 0.0);
                    // The grid may predate the flag: rebuild it for the
                    // surface looks, then render.
                    return Task::batch([self.recenter_3d(store), self.request_frame()]);
                }
                Task::none()
            }
            ViewerMsg::Ev3(e) => self.on_ev3(e, store),
            ViewerMsg::Grid3(g, m) => self.on_grid(g, m),
            ViewerMsg::Gpu3(on) => {
                self.v3.gpu = on;
                self.sync_pitch();
                self.regrid = true;
                if self.v3.blocks {
                    return self.request_blocks(store);
                }
                Task::none()
            }
            ViewerMsg::Frame3(h, a) => self.on_frame(h, a),
            ViewerMsg::Dpi3(f) => {
                if (f - self.v3.dpi).abs() < 0.001 {
                    return Task::none();
                }
                self.v3.dpi = f;
                self.request_frame()
            }
            ViewerMsg::Quality3(q) => {
                self.prefs.render_scale = q;
                self.request_frame()
            }
            ViewerMsg::Controls3(c) => {
                self.capture(false);
                self.prefs.controls = c;
                Task::none()
            }
            ViewerMsg::Move3(m) => {
                self.prefs.movement = m;
                Task::none()
            }
            ViewerMsg::LookSens3(v) => {
                self.prefs.look_sens = v;
                Task::none()
            }
            ViewerMsg::FlySpeed3(v) => {
                self.prefs.fly_speed = v;
                Task::none()
            }
            ViewerMsg::BlockDist3(r) => {
                self.prefs.block_distance = r;
                if self.v3.blocks { self.request_blocks(store) } else { Task::none() }
            }
            ViewerMsg::Far3(f) => {
                self.v3.cam.far = f;
                self.regrid = true;
                Task::none()
            }
            ViewerMsg::Detail3(b) => self.set_detail(b, store),
            ViewerMsg::Ruler(r) => {
                self.prefs.ruler = r;
                Task::none()
            }
            ViewerMsg::Entities(rect, e) => {
                self.ent_busy = false;
                if Some(rect) == self.ent_rect {
                    self.entities = e;
                }
                Task::none()
            }
            ViewerMsg::BGrid(r) => self.on_bgrid(r, store),
            ViewerMsg::Scale3(s) => {
                self.v3.cam.scale = s;
                self.request_frame()
            }
            ViewerMsg::Map(e) => self.on_event(e, store),
            ViewerMsg::MapUi(_) => self.ctl.refill(store).map(ViewerMsg::Tiles),
            ViewerMsg::Dim(d) => {
                let o = self.opts_dim(d);
                self.ctl.set_dim(d, o, store).map(ViewerMsg::Tiles)
            }
            ViewerMsg::Style(s) => {
                self.prefs.style.style = s;
                self.restyle();
                Task::none()
            }
            ViewerMsg::Azimuth(v) => {
                self.prefs.style.azimuth = v;
                self.restyle();
                Task::none()
            }
            ViewerMsg::Altitude(v) => {
                self.prefs.style.altitude = v;
                self.restyle();
                Task::none()
            }
            ViewerMsg::Strength(v) => {
                self.prefs.style.strength = v;
                self.restyle();
                Task::none()
            }
            ViewerMsg::Contour(v) => {
                self.prefs.style.contour = v;
                self.restyle();
                Task::none()
            }
            ViewerMsg::Shadows(v) => {
                self.prefs.style.shadows = v;
                self.restyle();
                Task::none()
            }
            ViewerMsg::Ambient(v) => {
                self.prefs.style.night_ambient = v;
                self.restyle();
                Task::none()
            }
            ViewerMsg::ChunkGrid(v) => {
                self.prefs.chunk_grid = v;
                Task::none()
            }
            ViewerMsg::Layer(l, on) => {
                match l {
                    Layer::RegionGrid => self.prefs.region_grid = on,
                    Layer::Slime => {
                        self.prefs.slime = on;
                        self.slime_rect = None;
                    }
                    Layer::Spawn => self.prefs.spawn = on,
                    Layer::Players => self.prefs.players = on,
                    Layer::Bookmarks => self.prefs.bookmarks = on,
                    Layer::Hud => self.prefs.hud = on,
                    Layer::Textures => self.prefs.textured = on,
                    Layer::Entities => {
                        self.prefs.entities = on;
                        self.ent_rect = None;
                    }
                    Layer::Border => self.prefs.border = on,
                    Layer::RegionLabels => self.prefs.region_labels = on,
                    Layer::BiomeTint => {
                        self.prefs.biome_tint = on;
                        return self.rerender(store);
                    }
                }
                Task::none()
            }
            ViewerMsg::Cave(on) => {
                self.cave = on;
                self.rerender(store)
            }
            ViewerMsg::Slicer(on) => {
                self.slicer = on.then_some(if self.ctl.dim == 1 { 100 } else { 64 });
                self.rerender(store)
            }
            ViewerMsg::SlicerMin(m) => {
                self.slicer_min = m;
                self.rerender(store)
            }
            ViewerMsg::SlicerY(y) => {
                self.slicer = Some(y);
                self.rerender(store)
            }
            ViewerMsg::GotoX(s) => {
                self.goto.0 = s;
                Task::none()
            }
            ViewerMsg::GotoZ(s) => {
                self.goto.1 = s;
                Task::none()
            }
            ViewerMsg::Goto => {
                // uNmINeD's location syntax: `spawn`/`s`, `player`/`p`, or
                // `x z` / `x y z` typed into one field.
                match self.goto.0.trim().to_lowercase().as_str() {
                    "spawn" | "s" => return self.update(sess, bookmarks, ViewerMsg::GotoSpawn),
                    "player" | "p" => return self.update(sess, bookmarks, ViewerMsg::GotoPlayer),
                    _ => {}
                }
                let nums: Vec<f64> = self.goto.0.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|p| p.trim().parse().ok()).collect();
                let (x, z) = match nums.len() {
                    2 => (Ok(nums[0]), Ok(nums[1])),
                    3 => (Ok(nums[0]), Ok(nums[2])),
                    _ => (self.goto.0.trim().parse::<f64>(), self.goto.1.trim().parse::<f64>()),
                };
                match (x, z) {
                    (Ok(x), Ok(z)) => {
                        self.center(x, z);
                        self.status = format!("centered on {x:.0}, {z:.0}");
                    }
                    _ => self.status = "enter block X and Z (numbers)".into(),
                }
                self.ctl.refill(store).map(ViewerMsg::Tiles)
            }
            ViewerMsg::Fit => {
                self.ctl.fit();
                self.ctl.refill(store).map(ViewerMsg::Tiles)
            }
            ViewerMsg::Zoom(f) => {
                let at = (self.ctl.camera.cx, self.ctl.camera.cz);
                self.ctl.camera.zoom_at(f, at);
                self.ctl.refill(store).map(ViewerMsg::Tiles)
            }
            ViewerMsg::BookmarkName(s) => {
                self.bookmark_name = s;
                Task::none()
            }
            ViewerMsg::AddBookmark => {
                let (x, z) = (self.ctl.camera.cx.round() as i32, self.ctl.camera.cz.round() as i32);
                let name = match self.bookmark_name.trim() {
                    "" => format!("Place {x}, {z}"),
                    n => n.to_string(),
                };
                bookmarks.push(Bookmark { world: self.origin.clone(), dim: self.ctl.dim, x, z, name: name.clone() });
                self.bookmark_name.clear();
                self.status = format!("bookmarked “{name}”");
                Task::none()
            }
            ViewerMsg::GoBookmark(i) => {
                let Some(b) = self.my_bookmarks(bookmarks).get(i).map(|(_, b)| (*b).clone()) else { return Task::none() };
                self.ctl.camera.zoom = self.ctl.camera.zoom.max(1.0);
                self.goto_dim(sess, b.dim, b.x as f64, b.z as f64)
            }
            ViewerMsg::RemoveBookmark(i) => {
                if let Some(global) = self.my_bookmarks(bookmarks).get(i).map(|(g, _)| *g) {
                    bookmarks.remove(global);
                }
                Task::none()
            }
            ViewerMsg::Export => Task::perform(
                async {
                    rfd::AsyncFileDialog::new().set_title("Export the current view").add_filter("PNG image", &["png"]).set_file_name("map.png").save_file().await.map(|h| h.path().to_path_buf())
                },
                ViewerMsg::ExportPicked,
            ),
            ViewerMsg::ExportPicked(None) => Task::none(),
            ViewerMsg::ExportPicked(Some(path)) => match crate::viewer_export::compose(&self.ctl) {
                Some((w, h, rgba)) => Task::perform(
                    blocking(move || crate::viewer_export::write_png(&path, w, h, &rgba).map(|()| format!("exported {w}×{h} view to {}", path.display()))),
                    ViewerMsg::Exported,
                ),
                None => {
                    self.status = "nothing rendered to export yet".into();
                    Task::none()
                }
            },
            ViewerMsg::Exported(r) => {
                self.status = r.unwrap_or_else(|e| format!("export failed: {e}"));
                Task::none()
            }
            ViewerMsg::TexCell(key, stamp, h) => {
                self.tex.insert(key, stamp, h);
                Task::none()
            }
            ViewerMsg::Markers(m) => {
                self.players = m.0.clone();
                self.seed = m.1;
                self.slime_rect = None;
                Task::none()
            }
            ViewerMsg::GotoSpawn => {
                let Some((x, z)) = sess.summary.spawn else {
                    self.status = "this world has no spawn point".into();
                    return Task::none();
                };
                self.status = format!("centered on spawn {x}, {z}");
                self.goto_dim(sess, 0, x as f64 + 0.5, z as f64 + 0.5)
            }
            ViewerMsg::GotoPlayer => {
                let target = self.players.iter().find(|p| p.dim == self.ctl.dim).or_else(|| self.players.first()).map(|p| (p.x, p.z, p.dim, p.label.clone()));
                let Some((x, z, dim, label)) = target else {
                    self.status = "no player markers in this world".into();
                    return Task::none();
                };
                self.status = format!("centered on {label}");
                self.goto_dim(sess, dim, x, z)
            }
            ViewerMsg::CopyTp => {
                let (x, z) = self.hover.map_or_else(
                    || (self.goto.0.trim().parse::<i32>().unwrap_or(0), self.goto.1.trim().parse::<i32>().unwrap_or(0)),
                    |(x, z)| (x.floor() as i32, z.floor() as i32),
                );
                let y = self.ctl.map.as_ref().and_then(|m| m.identify(x, z)).map(|(_, y, _)| y);
                let tp = match y {
                    Some(y) => format!("/tp {x} {y} {z}"),
                    None => format!("/tp {x} ~ {z}"),
                };
                self.status = format!("copied {tp}");
                iced::clipboard::write(tp)
            }
            ViewerMsg::ZoomReset => {
                self.ctl.camera.zoom = 1.0;
                self.ctl.refill(store).map(ViewerMsg::Tiles)
            }
            ViewerMsg::CycleChunkGrid => {
                self.prefs.chunk_grid = match self.prefs.chunk_grid {
                    None => Some(true),
                    Some(true) => Some(false),
                    Some(false) => None,
                };
                self.status = match self.prefs.chunk_grid {
                    None => "chunk grid: automatic".into(),
                    Some(true) => "chunk grid: always on".into(),
                    Some(false) => "chunk grid: off".into(),
                };
                Task::none()
            }
            ViewerMsg::Panel(p) => {
                self.prefs.set_panel(p);
                Task::none()
            }
            ViewerMsg::ToolsMenu(open) => {
                self.tools_menu = open;
                Task::none()
            }
            ViewerMsg::Key(k) => self.update(sess, bookmarks, k.into()),
            ViewerMsg::OpenIn(_) => Task::none(),
            ViewerMsg::Ext(m) => self.update_ext(sess, m),
        };
        self.plan_slime();
        if self.prefs.style.style == MapStyle::Biomes
            && let Some(map) = &self.ctl.map
            && map.tiles.len() != self.legend_tiles
        {
            self.legend_tiles = map.tiles.len();
            self.legend = map.biome_legend();
        }
        let regrid = if std::mem::take(&mut self.regrid) && self.v3.on { self.request_grid() } else { Task::none() };
        let ents = self.plan_entities(store);
        self.refresh_legends();
        let hl = self.plan_highlights(store);
        Task::batch([task, self.plan_textures(), regrid, ents, hl])
    }

    /// Query entities of the visible chunks when zoomed in enough.
    fn plan_entities(&mut self, store: Option<&Store>) -> Task<ViewerMsg> {
        if !self.prefs.entities || self.v3.on || self.ent_busy {
            return Task::none();
        }
        let (x0, z0, x1, z1) = self.ctl.camera.visible_blocks(self.ctl.view);
        let r = ((x0 / 16.0).floor() as i32, (z0 / 16.0).floor() as i32, (x1 / 16.0).ceil() as i32, (z1 / 16.0).ceil() as i32);
        let n = (r.2 - r.0 + 1) as i64 * (r.3 - r.1 + 1) as i64;
        if n > 1600 {
            self.entities = Arc::default();
            self.ent_rect = None;
            return Task::none();
        }
        if self.ent_rect == Some(r) {
            return Task::none();
        }
        let (Some(store), Some(map)) = (store.cloned(), self.ctl.map.as_ref()) else { return Task::none() };
        let chunks: Vec<(i32, i32)> = (r.1..=r.3).flat_map(|z| (r.0..=r.2).map(move |x| (x, z))).filter(|&(x, z)| map.chunk_present(x, z)).collect();
        self.ent_rect = Some(r);
        self.ent_busy = true;
        let dim = self.ctl.dim;
        Task::perform(
            blocking(move || match &store {
                Store::Bedrock(w) => omt_render::markers::bedrock_entities(w, dim, &chunks),
                Store::Java(w) => omt_render::markers::java_entities(w, dim, &chunks),
            }),
            move |e| ViewerMsg::Entities(r, Arc::new(e)),
        )
    }

    /// Whether the textured layer should show now.
    pub fn textured_active(&self) -> bool {
        !self.v3.on
            && self.prefs.textured && self.ctl.camera.zoom >= crate::viewer_tex::MIN_ZOOM && crate::textures::current().is_some() && self.ctl.map.as_ref().is_some_and(|m| m.level == 0)
    }

    /// Launch background builds of visible texture cells.
    fn plan_textures(&mut self) -> Task<ViewerMsg> {
        if !self.textured_active() {
            return Task::none();
        }
        let stamp = {
            let mut h = omt_render::source::Fnv::new();
            h.bytes(format!("{:?}{:?}{}{}{:?}{}", self.prefs.style, self.ctl.options(), crate::textures::generation(), self.ctl.dim, self.slicer, self.cave).as_bytes());
            h.finish()
        };
        self.tex.reset(stamp);
        let Some(map) = self.ctl.map.as_ref() else { return Task::none() };
        let jobs = self.tex.plan(map, self.ctl.camera.visible_blocks(self.ctl.view), 8);
        Task::batch(jobs.into_iter().map(|j| Task::perform(blocking(move || j.build()), |(k, s, h)| ViewerMsg::TexCell(k, s, h))))
    }

    fn center(&mut self, x: f64, z: f64) {
        self.ctl.camera.cx = x;
        self.ctl.camera.cz = z;
    }

    /// This world's bookmarks with their index in the global list.
    pub fn my_bookmarks<'b>(&self, all: &'b [Bookmark]) -> Vec<(usize, &'b Bookmark)> {
        all.iter().enumerate().filter(|(_, b)| b.world == self.origin).collect()
    }

    fn on_event(&mut self, e: MapEvent, store: Option<&Store>) -> Task<ViewerMsg> {
        match e {
            MapEvent::Pan { dx, dz } => {
                self.ctl.camera.cx += dx;
                self.ctl.camera.cz += dz;
                self.ctl.refill(store).map(ViewerMsg::Tiles)
            }
            MapEvent::Zoom { factor, at } => {
                self.ctl.camera.zoom_at(factor, at);
                self.ctl.refill(store).map(ViewerMsg::Tiles)
            }
            MapEvent::Resize(size) => self.ctl.on_viewport(size, store).map(ViewerMsg::Tiles),
            MapEvent::Hover { at } => {
                self.hover = Some(at);
                Task::none()
            }
            MapEvent::Drag { at, .. } => {
                self.hover = Some(at);
                // Left-drag measures a distance.
                if self.measuring
                    && let Some(m) = &mut self.measure
                {
                    m[1] = at;
                }
                Task::none()
            }
            MapEvent::Press { at, right: false, .. } if self.prefs.panel() == Some(PanelTab::Inspect) && self.ext.inspect.click_select => self.inspect_click(store, at).unwrap_or_else(Task::none),
            MapEvent::Press { at, right: false, .. } => {
                // Left click fills the go-to fields (copy coordinates) and
                // starts a measurement (kept only if the pointer moves).
                self.goto = (format!("{}", at.0.floor() as i64), format!("{}", at.1.floor() as i64));
                self.measure = Some([at, at]);
                self.measuring = true;
                Task::none()
            }
            MapEvent::Release { .. } => {
                self.measuring = false;
                if self.measure.is_some_and(|[a, b]| crate::mapview_overlay::distance(a, b) * self.ctl.camera.zoom as f64 <= 3.0) {
                    self.measure = None;
                }
                Task::none()
            }
            MapEvent::Press { .. } => Task::none(),
        }
    }
}
