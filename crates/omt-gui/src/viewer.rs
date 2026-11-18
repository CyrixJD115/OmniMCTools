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
use omt_render::style::{MapStyle, StyleParams};
use omt_render::RenderOptions;
use serde::{Deserialize, Serialize};

use crate::mapdata::{MapController, MapMsg};
use crate::mapview::MapEvent;
use crate::session::{Store, WorldSession, blocking};

/// Remembered viewer look (Settings TOML).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewerPrefs {
    pub style: StyleParams,
    /// `None` = automatic.
    pub chunk_grid: Option<bool>,
    pub region_grid: bool,
    pub slime: bool,
    pub spawn: bool,
    pub players: bool,
    pub bookmarks: bool,
    pub hud: bool,
    pub biome_tint: bool,
    /// Real block textures when zoomed in (needs a texture pack).
    pub textured: bool,
    /// Entity pins when zoomed in.
    pub entities: bool,
    /// Rulers along the map edges.
    pub ruler: crate::mapview_overlay::Ruler,
    /// 3D render resolution relative to the screen's pixels (1 = native).
    pub render_scale: f32,
    /// 3D "Blocks" render distance (blocks from the camera).
    pub block_distance: i32,
    /// 3D mouse controls.
    pub controls: crate::viewer_3d_input::Controls3,
    /// 3D collision (Normal stays above ground, Spectator flies through).
    pub movement: crate::viewer_3d_input::Move3,
    /// Look sensitivity multiplier.
    pub look_sens: f32,
    /// Keyboard fly speed (blocks / second; Ctrl × 2.5).
    pub fly_speed: f32,
}

impl Default for ViewerPrefs {
    fn default() -> Self {
        Self { style: StyleParams::default(), chunk_grid: None, region_grid: false, slime: false, spawn: true, players: true, bookmarks: true, hud: true, biome_tint: true, textured: true, entities: true, ruler: crate::mapview_overlay::Ruler::Blocks, render_scale: 1.0,
            block_distance: 192,
            controls: crate::viewer_3d_input::Controls3::Drag,
            movement: crate::viewer_3d_input::Move3::Normal,
            look_sens: 1.0,
            fly_speed: 20.0,
        }
    }
}

/// A saved place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub world: PathBuf,
    pub dim: u8,
    pub x: i32,
    pub z: i32,
    pub name: String,
}

/// Layer toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    RegionGrid,
    Slime,
    Spawn,
    Players,
    Bookmarks,
    Hud,
    BiomeTint,
    Textures,
    Entities,
}

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
    ChunkGrid(Option<bool>),
    Layer(Layer, bool),
    Cave(bool),
    Slicer(bool),
    SlicerY(i32),
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
}

pub struct ViewerState {
    pub ctl: MapController,
    pub prefs: ViewerPrefs,
    pub cave: bool,
    pub slicer: Option<i32>,
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
}

impl ViewerState {
    pub fn new(sess: &WorldSession, prefs: ViewerPrefs) -> Self {
        let mut ctl = MapController::new(0, Self::opts_for(0, false, None, prefs.biome_tint));
        ctl.post = Some(prefs.style);
        Self {
            ctl,
            prefs,
            cave: false,
            slicer: None,
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
            legend: Vec::new(),
            legend_tiles: usize::MAX,
            entities: Arc::default(),
            ent_rect: None,
            ent_busy: false,
            measure: None,
            measuring: false,
        }
    }

    /// Render options: no baked relief (the style lights the map).
    fn opts_for(dim: u8, cave: bool, slicer: Option<i32>, tint: bool) -> RenderOptions {
        let base = RenderOptions::for_dimension(dim);
        RenderOptions { relief: false, cave, max_y: slicer.or(base.max_y), biome_tint: tint, ..base }
    }

    fn opts(&self) -> RenderOptions {
        Self::opts_for(self.ctl.dim, self.cave, self.slicer, self.prefs.biome_tint)
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
        Task::batch([tiles, markers])
    }

    pub fn status_line(&self) -> String {
        match (self.status.is_empty(), self.ctl.status.is_empty()) {
            (true, _) => self.ctl.status.clone(),
            (false, true) => self.status.clone(),
            (false, false) => format!("{} — {}", self.status, self.ctl.status),
        }
    }

    fn rerender(&mut self, store: Option<&Store>) -> Task<ViewerMsg> {
        let o = self.opts();
        self.ctl.set_options(o, store).map(ViewerMsg::Tiles)
    }

    fn restyle(&mut self) {
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
                    let enter = self.set_3d(true, store);
                    let detail = if mode == "blocks" { self.set_detail(true, store) } else { Task::none() };
                    return Task::batch([t, enter, detail]);
                }
                t
            }
            ViewerMsg::Mode3D(on) => self.set_3d(on, store),
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
                let o = Self::opts_for(d, self.cave, self.slicer, self.prefs.biome_tint);
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
                match (self.goto.0.trim().parse::<f64>(), self.goto.1.trim().parse::<f64>()) {
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
                let mut tasks = Vec::new();
                if b.dim != self.ctl.dim {
                    let o = Self::opts_for(b.dim, self.cave, self.slicer, self.prefs.biome_tint);
                    tasks.push(self.ctl.set_dim(b.dim, o, store).map(ViewerMsg::Tiles));
                }
                self.center(b.x as f64, b.z as f64);
                self.ctl.camera.zoom = self.ctl.camera.zoom.max(1.0);
                tasks.push(self.ctl.refill(store).map(ViewerMsg::Tiles));
                Task::batch(tasks)
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
        Task::batch([task, self.plan_textures(), regrid, ents])
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
