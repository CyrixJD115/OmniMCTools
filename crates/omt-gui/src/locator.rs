//! Chunk Locator (UMT editor side-tab): find chunks with the 2D viewer —
//! biome tint, textures, heightmap/water shading, Slicer, dimension
//! switcher, goto coordinates, click to identify, and open the chunk's
//! data in the NBT editor.

use iced::widget::{button, column, container, row, slider, space, text, text_input};
use iced::{Element, Length, Task};

use crate::mapdata::{MapController, MapMsg};
use crate::mapview::{MapEvent, MapView, Overlay};
use crate::session::Store;
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
    Tiles(MapMsg),
    /// Map toolbar (applied to settings by the app, then refills here).
    MapUi(crate::mapdata::MapUiMsg),
    Open(OpenWhat),
    Fit,
}

/// A request from the locator to open a chunk's data: (what, dim, x, z).
pub type OpenRequest = (OpenWhat, u8, i32, i32);

pub struct Locator {
    pub ctl: MapController,
    slicer: Option<i32>,
    slicer_draft: i32,
    goto: String,
    pub selected: Option<(i32, i32)>,
    selected_block: Option<(i32, i32)>,
    hover: Option<(f64, f64)>,
    nether_default: i32,
}

impl Locator {
    pub fn new(nether_default: i32) -> Self {
        let mut l = Self {
            ctl: MapController::new(0, omt_render::RenderOptions::default()),
            slicer: None,
            slicer_draft: 319,
            goto: String::new(),
            selected: None,
            selected_block: None,
            hover: None,
            nether_default,
        };
        l.ctl = MapController::new(0, l.render_options(0));
        l
    }

    fn render_options(&self, dim: u8) -> omt_render::RenderOptions {
        let mut opts = omt_render::RenderOptions::for_dimension(dim);
        if dim == 1 {
            opts.max_y = Some(self.nether_default);
        }
        if self.slicer.is_some() {
            opts.max_y = self.slicer;
        }
        opts
    }

    pub fn is_loaded(&self) -> bool {
        self.ctl.map.is_some() || self.ctl.loading
    }

    pub fn load(&mut self, store: Option<Store>) -> Task<LocMsg> {
        self.ctl.load(store.as_ref(), false).map(LocMsg::Tiles)
    }

    /// Returns a task and, for `Open`, the request for the session.
    pub fn update(&mut self, msg: LocMsg, store: Option<Store>) -> (Task<LocMsg>, Option<OpenRequest>) {
        let st = store.as_ref();
        let task = match msg {
            LocMsg::Map(ev) => match ev {
                MapEvent::Pan { dx, dz } => {
                    self.ctl.camera.cx += dx;
                    self.ctl.camera.cz += dz;
                    self.ctl.refill(st)
                }
                MapEvent::Zoom { factor, at } => {
                    self.ctl.camera.zoom_at(factor, at);
                    self.ctl.refill(st)
                }
                MapEvent::Resize(size) => self.ctl.on_viewport(size, st),
                MapEvent::Hover { at } => {
                    self.hover = Some(at);
                    Task::none()
                }
                MapEvent::Press { at, right: false, .. } => {
                    let (bx, bz) = (at.0.floor() as i32, at.1.floor() as i32);
                    self.selected = Some((bx.div_euclid(16), bz.div_euclid(16)));
                    self.selected_block = Some((bx, bz));
                    Task::none()
                }
                _ => Task::none(),
            },
            LocMsg::Dim(d) => {
                if d != self.ctl.dim {
                    self.slicer = None;
                    self.selected = None;
                    let opts = self.render_options(d);
                    self.ctl.set_dim(d, opts, st)
                } else {
                    Task::none()
                }
            }
            LocMsg::Slicer(v) => {
                self.slicer_draft = v;
                Task::none()
            }
            LocMsg::SlicerCommit => {
                self.slicer = Some(self.slicer_draft);
                let opts = self.render_options(self.ctl.dim);
                self.ctl.set_options(opts, st)
            }
            LocMsg::SlicerOff => {
                self.slicer = None;
                let opts = self.render_options(self.ctl.dim);
                self.ctl.set_options(opts, st)
            }
            LocMsg::Goto(s) => {
                self.goto = s;
                Task::none()
            }
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
                        self.ctl.status = "Goto expects `x z` or `x y z` block coordinates".into();
                        return (Task::none(), None);
                    }
                };
                self.ctl.camera.cx = x;
                self.ctl.camera.cz = z;
                self.ctl.camera.zoom = self.ctl.camera.zoom.max(2.0);
                self.selected = Some(((x.floor() as i32).div_euclid(16), (z.floor() as i32).div_euclid(16)));
                self.selected_block = Some((x.floor() as i32, z.floor() as i32));
                self.ctl.refill(st)
            }
            LocMsg::Tiles(m) => self.ctl.on_msg(m, st),
            LocMsg::MapUi(_) => self.ctl.refill(st),
            LocMsg::Open(what) => {
                if let Some((x, z)) = self.selected {
                    return (Task::none(), Some((what, self.ctl.dim, x, z)));
                }
                Task::none()
            }
            LocMsg::Fit => {
                self.ctl.fit();
                self.ctl.refill(st)
            }
        };
        (task.map(LocMsg::Tiles), None)
    }

    pub fn view(&self, bedrock: bool) -> Element<'_, LocMsg> {
        let caption = match self.hover {
            Some((x, z)) => {
                let (bx, bz) = (x.floor() as i32, z.floor() as i32);
                let id = self.ctl.map.as_ref().and_then(|m| m.identify(bx, bz)).map(|(n, y, approx)| format!(" · {}{n} @ Y {y}", if approx { "≈ " } else { "" })).unwrap_or_default();
                format!("{bx}:{bz} · chunk {}:{}{id}", bx.div_euclid(16), bz.div_euclid(16))
            }
            None => String::new(),
        };
        let overlay = Overlay { marker: self.selected, caption, ..Default::default() };
        let viewer: Element<'_, LocMsg> = match &self.ctl.map {
            Some(map) => crate::mapview::element(MapView { map, camera: self.ctl.camera, overlay, on_event: Box::new(LocMsg::Map) }),
            None => container(text(if self.ctl.loading { self.ctl.status.as_str() } else { "No map" }).color(style::muted())).center(Length::Fill).into(),
        };
        let slicer_label = match self.slicer {
            Some(y) => format!("Slicer: Y ≤ {y}"),
            None => "Slicer: off".into(),
        };
        let top = row![
            crate::pruner::dim_switcher(self.ctl.dim, LocMsg::Dim),
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
                let ident = self
                    .ctl
                    .map
                    .as_ref()
                    .and_then(|m| m.identify(bx, bz))
                    .map(|(n, y, approx)| {
                        if approx { format!("Surface ≈ {n} at Y {y} (zoom in for exact)") } else { format!("Surface: {n} at Y {y}") }
                    })
                    .unwrap_or_else(|| "No data here".into());
                let present = self.ctl.map.as_ref().is_some_and(|m| m.chunk_present(cx, cz));
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
                    text(if present { ident } else { "Chunk not generated".into() }).size(12).color(style::muted()),
                    actions,
                ]
                .spacing(6)
                .into()
            }
            _ => text("Click the map to select a chunk.").size(12).color(style::muted()).into(),
        };
        let map_bar = row![text("Map").size(12).color(style::muted()), crate::mapdata::controls(LocMsg::MapUi)].spacing(8).align_y(iced::Alignment::Center);
        column![
            top,
            map_bar,
            viewer,
            container(column![slicer, info, text(self.ctl.status.clone()).size(11).color(style::hint())].spacing(6)).padding(8).style(style::card()),
        ]
        .spacing(6)
        .padding(8)
        .into()
    }
}
