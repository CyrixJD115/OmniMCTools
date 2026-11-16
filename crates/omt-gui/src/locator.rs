//! Chunk Locator (UMT editor side-tab): find chunks with the 2D viewer —
//! biome tint, textures, heightmap/water shading, Slicer, dimension
//! switcher, goto coordinates, click to identify, and open the chunk's
//! data in the NBT editor.

use iced::widget::{button, canvas, column, container, row, slider, space, text, text_input};
use iced::{Element, Length, Size, Task};

use crate::mapdata::{self, MapBuild, MapData, MapScan, TileSource, MAX_INFLIGHT};
use crate::mapview::{Camera, MapEvent, MapView, Overlay};
use crate::session::{Store, blocking};
use crate::style::{self, Btn};

/// What the user asked to open for the selected chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenWhat {
    /// Java chunk NBT (region) / Bedrock block entities.
    Chunk,
    /// Java entity-region chunk / Bedrock actors.
    Entities,
    /// Bedrock block entities (0x31).
    BlockEntities,
}

#[derive(Debug, Clone)]
pub enum LocMsg {
    Map(MapEvent),
    Dim(u8),
    Slicer(i32),
    SlicerCommit,
    SlicerOff,
    Goto(String),
    GotoSubmit,
    ScanReady(u8, Result<MapScan, String>),
    Tile(u8, (i32, i32), Result<MapBuild, String>),
    Open(OpenWhat),
    Fit,
}

/// A request from the locator to open a chunk's data: (what, dim, x, z).
pub type OpenRequest = (OpenWhat, u8, i32, i32);

pub struct Locator {
    pub dim: u8,
    pub camera: Camera,
    pub map: Option<MapData>,
    source: Option<TileSource>,
    inflight: HashSet<(i32, i32)>,
    loading: bool,
    slicer: Option<i32>,
    slicer_draft: i32,
    goto: String,
    pub selected: Option<(i32, i32)>,
    selected_block: Option<(i32, i32)>,
    hover: Option<(f64, f64)>,
    pub status: String,
    nether_default: i32,
    fitted: bool,
}

use std::collections::HashSet;

impl Locator {
    pub fn new(nether_default: i32) -> Self {
        Self {
            dim: 0,
            camera: Camera::default(),
            map: None,
            source: None,
            inflight: HashSet::new(),
            loading: false,
            slicer: None,
            slicer_draft: 319,
            goto: String::new(),
            selected: None,
            selected_block: None,
            hover: None,
            status: String::new(),
            nether_default,
            fitted: false,
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.map.is_some() || self.loading
    }

    pub fn load(&mut self, store: Option<Store>) -> Task<LocMsg> {
        let Some(store) = store else { return Task::none() };
        self.loading = true;
        let dim = self.dim;
        self.status = "Scanning chunks…".into();
        let s2 = store.clone();
        Task::perform(blocking(move || crate::mapdata::scan(&s2, dim)), move |r| {
            LocMsg::ScanReady(dim, r)
        })
    }

    /// Build the tiles the camera can see (on demand), offload far ones, and
    /// keep at most `MAX_INFLIGHT` background builds running.
    fn refill(&mut self) -> Task<LocMsg> {
        let Some(map) = &mut self.map else { return Task::none() };
        let Some(src) = self.source.clone() else { return Task::none() };
        let view = Size::new(900.0, 640.0);
        let (bx0, bz0, bx1, bz1) = self.camera.visible_blocks(view);
        let mut want = mapdata::tile_range(bx0, bz0, bx1, bz1);
        // One-tile margin so panning never shows holes at the edges.
        want.0 -= 1;
        want.1 -= 1;
        want.2 += 1;
        want.3 += 1;
        let center = (
            ((bx0 + bx1) / 2.0).floor() as i32,
            ((bz0 + bz1) / 2.0).floor() as i32,
        );
        let center_tile = (center.0.div_euclid(mapdata::TILE_CHUNKS), center.1.div_euclid(mapdata::TILE_CHUNKS));
        let dropped = map.prune_cache(want, center_tile);
        if dropped > 0 {
            self.status = format!("{} tiles cached (offloaded {dropped} off-screen)", map.tiles.len());
        }
        let slots = MAX_INFLIGHT.saturating_sub(self.inflight.len());
        if slots == 0 {
            return Task::none();
        }
        let next = mapdata::refill_plan(&map.tiles, &self.inflight, want, center_tile, slots);
        let dim = self.dim;
        let mut opts = omt_render::RenderOptions::for_dimension(dim);
        if dim == 1 {
            opts.max_y = Some(self.nether_default);
        }
        if self.slicer.is_some() {
            opts.max_y = self.slicer;
        }
        let mut tasks = Vec::new();
        for tile in next {
            self.inflight.insert(tile);
            let src = src.clone();
            tasks.push(Task::perform(
                blocking(move || mapdata::build_tile(&src, dim, opts, tile.0, tile.1)),
                move |r| LocMsg::Tile(dim, tile, r),
            ));
        }
        if !tasks.is_empty() {
            self.status = format!(
                "{} tiles built · {} rendering… · {} chunks total",
                map.tiles.len(),
                self.inflight.len(),
                map.chunk_total()
            );
        }
        Task::batch(tasks)
    }

    /// Returns a task and, for `Open`, the request for the session.
    pub fn update(&mut self, msg: LocMsg, store: Option<Store>) -> (Task<LocMsg>, Option<OpenRequest>) {
        match msg {
            LocMsg::Map(ev) => match ev {
                MapEvent::Pan { dx, dz } => {
                    self.camera.cx += dx;
                    self.camera.cz += dz;
                    return (self.refill(), None);
                }
                MapEvent::Zoom { factor, at } => {
                    self.camera.zoom_at(factor, at);
                    return (self.refill(), None);
                }
                MapEvent::Hover { at } => self.hover = Some(at),
                MapEvent::Press { at, right: false, .. } => {
                    let (bx, bz) = (at.0.floor() as i32, at.1.floor() as i32);
                    self.selected = Some((bx.div_euclid(16), bz.div_euclid(16)));
                    self.selected_block = Some((bx, bz));
                }
                _ => {}
            },
            LocMsg::Dim(d) => {
                if d != self.dim {
                    self.dim = d;
                    self.slicer = None;
                    self.map = None;
                    self.selected = None;
                    self.fitted = false;
                    return (self.load(store), None);
                }
            }
            LocMsg::Slicer(v) => self.slicer_draft = v,
            LocMsg::SlicerCommit => {
                self.slicer = Some(self.slicer_draft);
                return (self.load(store), None);
            }
            LocMsg::SlicerOff => {
                self.slicer = None;
                return (self.load(store), None);
            }
            LocMsg::Goto(s) => self.goto = s,
            LocMsg::GotoSubmit => {
                let nums: Vec<f64> = self
                    .goto
                    .split([',', ' ', ':'])
                    .filter(|s| !s.is_empty())
                    .filter_map(|s| s.parse().ok())
                    .collect();
                // "x z" or "x y z" block coordinates (UMT %1:%2 / %1:%2:%3).
                let (x, z) = match nums.as_slice() {
                    [x, z] => (*x, *z),
                    [x, _, z] => (*x, *z),
                    _ => {
                        self.status = "Goto expects `x z` or `x y z` block coordinates".into();
                        return (Task::none(), None);
                    }
                };
                self.camera.cx = x;
                self.camera.cz = z;
                self.camera.zoom = self.camera.zoom.max(2.0);
                self.selected = Some(((x.floor() as i32).div_euclid(16), (z.floor() as i32).div_euclid(16)));
                self.selected_block = Some((x.floor() as i32, z.floor() as i32));
            }
            LocMsg::ScanReady(dim, res) => {
                if dim != self.dim {
                    return (Task::none(), None);
                }
                self.loading = false;
                match res {
                    Ok(scan) => {
                        self.status = format!(
                            "{} chunks · scanned in {:.1}s — rendering visible tiles…",
                            scan.chunks.len(),
                            scan.millis as f32 / 1000.0
                        );
                        self.source = store.as_ref().map(crate::mapdata::tile_source).transpose().ok().flatten();
                        self.map = Some(MapData::with_scan(scan));
                        if !self.fitted && let Some(bounds) = self.map.as_ref().and_then(MapData::chunk_bounds) {
                            self.camera = Camera::fit(bounds, Size::new(900.0, 640.0));
                            self.fitted = true;
                        }
                        return (self.refill(), None);
                    }
                    Err(e) => self.status = format!("render failed: {e}"),
                }
            }
            LocMsg::Tile(dim, tile, res) => {
                self.inflight.remove(&tile);
                if dim != self.dim || res.is_err() {
                    if let Err(e) = &res {
                        self.status = format!("tile render failed: {e}");
                    }
                    return (self.refill(), None);
                }
                if let Some(map) = &mut self.map
                    && let Ok(build) = res
                {
                    let errs = build.errors;
                    map.insert(build);
                    if errs > 0 {
                        self.status = format!("{} tile unreadable chunks (shown as holes)", errs);
                    }
                }
                return (self.refill(), None);
            }
            LocMsg::Open(what) => {
                if let Some((x, z)) = self.selected {
                    return (Task::none(), Some((what, self.dim, x, z)));
                }
            }
            LocMsg::Fit => {
                if let Some(b) = self.map.as_ref().and_then(MapData::chunk_bounds) {
                    self.camera = Camera::fit(b, Size::new(900.0, 640.0));
                }
            }
        }
        (Task::none(), None)
    }

    pub fn view(&self, bedrock: bool) -> Element<'_, LocMsg> {
        let caption = match self.hover {
            Some((x, z)) => {
                let (bx, bz) = (x.floor() as i32, z.floor() as i32);
                let id = self.map.as_ref().and_then(|m| m.identify(bx, bz)).map(|(n, y)| format!(" · {n} @ Y {y}")).unwrap_or_default();
                format!("{bx}:{bz} · chunk {}:{}{id}", bx.div_euclid(16), bz.div_euclid(16))
            }
            None => String::new(),
        };
        let overlay = Overlay { red: Vec::new(), outlines: Vec::new(), marker: self.selected, caption };
        let viewer: Element<'_, LocMsg> = match &self.map {
            Some(map) => canvas(MapView { map, camera: self.camera, overlay, on_event: Box::new(LocMsg::Map) })
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => container(text(if self.loading { "Rendering…" } else { "No map" }).color(style::MUTED)).center(Length::Fill).into(),
        };
        let slicer_label = match self.slicer {
            Some(y) => format!("Slicer: Y ≤ {y}"),
            None => "Slicer: off".into(),
        };
        let top = row![
            crate::pruner::dim_switcher(self.dim, LocMsg::Dim),
            space::horizontal(),
            text_input("goto x z", &self.goto).on_input(LocMsg::Goto).on_submit(LocMsg::GotoSubmit).width(150).style(style::input()),
            button(text("Go").size(12)).on_press(LocMsg::GotoSubmit).style(style::button_style(Btn::Standard)),
            button(text("Fit").size(12)).on_press(LocMsg::Fit).style(style::button_style(Btn::Ghost)),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);
        let slicer = row![
            text(slicer_label).size(12).width(110),
            slider(-64..=319, self.slicer_draft, LocMsg::Slicer).on_release(LocMsg::SlicerCommit).width(Length::Fill),
            text(format!("{}", self.slicer_draft)).size(12).width(36),
            button(text("Off").size(11)).on_press(LocMsg::SlicerOff).style(style::button_style(Btn::Ghost)),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        let info: Element<'_, LocMsg> = match (self.selected, self.selected_block) {
            (Some((cx, cz)), Some((bx, bz))) => {
                let ident = self.map.as_ref().and_then(|m| m.identify(bx, bz)).map(|(n, y)| format!("Surface: {n} at Y {y}")).unwrap_or_else(|| "No data here".into());
                let present = self.map.as_ref().is_some_and(|m| m.chunk_present(cx, cz));
                let mut actions = row![].spacing(6);
                if present {
                    if bedrock {
                        actions = actions
                            .push(button(text("Block Entities").size(12)).on_press(LocMsg::Open(OpenWhat::BlockEntities)).style(style::button_style(Btn::Primary)))
                            .push(button(text("Entities").size(12)).on_press(LocMsg::Open(OpenWhat::Entities)).style(style::button_style(Btn::Standard)));
                    } else {
                        actions = actions
                            .push(button(text("Open Chunk NBT").size(12)).on_press(LocMsg::Open(OpenWhat::Chunk)).style(style::button_style(Btn::Primary)))
                            .push(button(text("Entities").size(12)).on_press(LocMsg::Open(OpenWhat::Entities)).style(style::button_style(Btn::Standard)));
                    }
                }
                column![
                    text(format!("Chunk {cx}:{cz} · Block {bx}:{bz} · Region r.{}.{}", cx.div_euclid(32), cz.div_euclid(32))).size(13),
                    text(if present { ident } else { "Chunk not generated".into() }).size(12).color(style::MUTED),
                    actions,
                ]
                .spacing(6)
                .into()
            }
            _ => text("Click the map to select a chunk.").size(12).color(style::MUTED).into(),
        };
        column![
            top,
            viewer,
            container(column![slicer, info, text(self.status.clone()).size(11).color(style::HINT)].spacing(6)).padding(8).style(style::card()),
        ]
        .spacing(6)
        .padding(8)
        .into()
    }
}
