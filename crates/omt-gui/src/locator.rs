//! Chunk Locator (UMT editor side-tab): find chunks with the 2D viewer —
//! biome tint, textures, heightmap/water shading, Slicer, dimension
//! switcher, goto coordinates, click to identify, and open the chunk's
//! data in the NBT editor.

use iced::widget::{button, canvas, column, container, row, slider, space, text, text_input};
use iced::{Element, Length, Size, Task};

use crate::mapdata::{MapBuild, MapData};
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
    Ready(u8, Result<std::sync::Arc<std::sync::Mutex<Option<MapBuild>>>, String>),
    Open(OpenWhat),
    Fit,
}

/// A request from the locator to open a chunk's data: (what, dim, x, z).
pub type OpenRequest = (OpenWhat, u8, i32, i32);

pub struct Locator {
    pub dim: u8,
    pub camera: Camera,
    pub map: Option<MapData>,
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

impl Locator {
    pub fn new(nether_default: i32) -> Self {
        Self {
            dim: 0,
            camera: Camera::default(),
            map: None,
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
        let mut opts = omt_render::RenderOptions::for_dimension(dim);
        if dim == 1 {
            opts.max_y = Some(self.nether_default);
        }
        if self.slicer.is_some() {
            opts.max_y = self.slicer;
        }
        self.slicer_draft = opts.max_y.unwrap_or(319);
        self.status = "Rendering…".into();
        Task::perform(blocking(move || crate::mapdata::build(store, dim, opts)), move |r| {
            LocMsg::Ready(dim, r.map(|b| std::sync::Arc::new(std::sync::Mutex::new(Some(b)))))
        })
    }

    /// Returns a task and, for `Open`, the request for the session.
    pub fn update(&mut self, msg: LocMsg, store: Option<Store>) -> (Task<LocMsg>, Option<OpenRequest>) {
        match msg {
            LocMsg::Map(ev) => match ev {
                MapEvent::Pan { dx, dz } => {
                    self.camera.cx += dx;
                    self.camera.cz += dz;
                }
                MapEvent::Zoom { factor, at } => self.camera.zoom_at(factor, at),
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
            LocMsg::Ready(dim, res) => {
                if dim == self.dim {
                    self.loading = false;
                    match res {
                        Ok(slot) => {
                            if let Some(b) = slot.lock().ok().and_then(|mut g| g.take()) {
                                let data = MapData::from_build(b);
                                self.status = match data.errors {
                                    0 => format!("{} chunks · {:.1}s", data.chunks.len(), data.millis as f32 / 1000.0),
                                    n => format!(
                                        "{} chunks · {n} UNREADABLE (shown as holes) · {:.1}s",
                                        data.chunks.len(),
                                        data.millis as f32 / 1000.0
                                    ),
                                };
                                if !self.fitted && let Some(bounds) = data.chunk_bounds() {
                                    self.camera = Camera::fit(bounds, Size::new(900.0, 640.0));
                                    self.fitted = true;
                                }
                                self.map = Some(data);
                            }
                        }
                        Err(e) => self.status = format!("render failed: {e}"),
                    }
                }
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
                let present = self.map.as_ref().is_some_and(|m| m.chunks.contains(&(cx, cz)));
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
