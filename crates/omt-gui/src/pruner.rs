//! Pruner app (UMT "Pruner"): selection controls on the left, the 2D world
//! viewer on the right. Tools: Box · Circle · Polygon · Split · Time ·
//! Optimize. Each drawn shape becomes a layer; layers union; selected
//! chunks show red; Prune deletes them from the workspace copy (Save writes
//! back, Close discards).

use std::collections::HashMap;

use iced::widget::{button, column, container, pick_list, row, scrollable, slider, space, text, text_input, toggler, tooltip};
use iced::{Element, Length, Task};
use omt_prune::{Layer, Selection, Shape, TimeOp};
use omt_render::style::MapStyle;
use omt_world::Dimension;

use crate::mapdata::{MapController, MapData, MapMsg};
use crate::mapview::{MapEvent, MapView, Outline, Overlay};
use crate::session::{Store, WorldSession, blocking};
use crate::style::{self, Btn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Box,
    Circle,
    Polygon,
    Split,
}

impl Tool {
    const ALL: [Tool; 4] = [Tool::Box, Tool::Circle, Tool::Polygon, Tool::Split];
    /// Glyph for the tool rail button.
    fn rail_glyph(self) -> &'static str {
        match self {
            Self::Box => "▭",
            Self::Circle => "◯",
            Self::Polygon => "⬠",
            Self::Split => "⟋",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Box => "Left-click and drag to set position and size.",
            Self::Circle => "Left-click and drag to set the center and radius.",
            Self::Polygon => "Left-click to add points; right-click to close the shape.",
            Self::Split => "Left-click and drag to set position and angle; selects one side.",
        }
    }
}

#[derive(Debug, Clone)]
pub enum PrunerMsg {
    Map(MapEvent),
    Tool(Tool),
    Dim(u8),
    Tiles(MapMsg),
    /// Map toolbar (applied to settings by the app, then refills here).
    MapUi(crate::mapdata::MapUiMsg),
    /// Map look (style, grids, ruler, HUD) — saved by the app.
    Look(crate::maplook::LookMsg),
    InhabitedReady(Result<HashMap<(i32, i32), i64>, String>),
    SelectLayer(usize),
    ToggleInvert(usize),
    ToggleEnable(usize),
    DeleteLayer(usize),
    ClearLayers,
    TimeOp(TimeOp),
    TimeValue(String),
    AddTime,
    Prune,
    ConfirmPrune,
    CancelPrune,
    PruneDone(Result<String, String>),
    Optimize,
    OptimizeDone(Result<String, String>),
    FitView,
    /// Show/hide the rendering row (quality · detail · sun).
    ToggleRender,
    /// Show/hide the time-selection controls.
    ToggleTime,
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    /// Drawing a new shape from an anchor point.
    New((f64, f64)),
    /// Moving control point `handle` of layer `layer`.
    Handle { layer: usize, handle: usize },
}

pub struct PrunerState {
    pub ctl: MapController,
    tool: Tool,
    pub selection: Selection,
    active: Option<usize>,
    drag: Option<Drag>,
    polygon: Vec<(f64, f64)>,
    inhabited: Option<HashMap<(i32, i32), i64>>,
    red: Vec<(i32, i32)>,
    time_op: TimeOp,
    time_value: String,
    confirm: bool,
    busy: bool,
    /// The rendering row (quality/detail/sun) is expanded.
    render_open: bool,
    /// The time-selection controls are expanded.
    time_open: bool,
    pub status: String,
    hover: Option<(f64, f64)>,
    /// Map look (style, grids, ruler, HUD).
    pub look: crate::maplook::MapLook,
}

/// Pruner render options: relief off (the map look lights the tiles).
fn pruner_opts(dim: u8) -> omt_render::RenderOptions {
    omt_render::RenderOptions { relief: false, ..omt_render::RenderOptions::for_dimension(dim) }
}

impl Default for PrunerState {
    fn default() -> Self {
        Self {
            ctl: MapController::new(0, pruner_opts(0)),
            look: crate::maplook::MapLook::default(),
            tool: Tool::Box,
            selection: Selection::default(),
            active: None,
            drag: None,
            polygon: Vec::new(),
            inhabited: None,
            red: Vec::new(),
            time_op: TimeOp::Less,
            time_value: "1200".into(),
            confirm: false,
            busy: false,
            render_open: false,
            time_open: true,
            status: String::new(),
            hover: None,
        }
    }
}

fn snap_floor(v: f64, snap: bool) -> f64 {
    if snap { (v / 16.0).floor() * 16.0 } else { v.floor() }
}

fn snap_round(v: f64, snap: bool) -> f64 {
    if snap { (v / 16.0).round() * 16.0 } else { v.floor() }
}

/// Control points of a layer's shape (block coordinates).
fn handles(shape: &Shape) -> Vec<(f64, f64)> {
    match shape {
        Shape::Box { x0, z0, x1, z1 } => vec![(*x0, *z0), (*x1 + 1.0, *z1 + 1.0)],
        Shape::Circle { cx, cz, radius } => vec![(*cx, *cz), (*cx + *radius, *cz)],
        Shape::Polygon { points } => points.clone(),
        Shape::Split { x, z, angle_deg } => {
            let a = angle_deg.to_radians();
            vec![(*x, *z), (*x + 64.0 * a.cos(), *z + 64.0 * a.sin())]
        }
        Shape::Time { .. } => Vec::new(),
    }
}

fn outline(shape: &Shape) -> Option<(Vec<(f64, f64)>, bool)> {
    match shape {
        Shape::Box { x0, z0, x1, z1 } => {
            let (a, b, c, d) = (x0.min(*x1), z0.min(*z1), x0.max(*x1) + 1.0, z0.max(*z1) + 1.0);
            Some((vec![(a, b), (c, b), (c, d), (a, d)], true))
        }
        Shape::Circle { cx, cz, radius } => Some((
            (0..64)
                .map(|i| {
                    let t = i as f64 / 64.0 * std::f64::consts::TAU;
                    (cx + radius * t.cos(), cz + radius * t.sin())
                })
                .collect(),
            true,
        )),
        Shape::Polygon { points } => Some((points.clone(), true)),
        Shape::Split { x, z, angle_deg } => {
            let a = angle_deg.to_radians();
            let (dx, dz) = (a.cos() * 1.0e6, a.sin() * 1.0e6);
            Some((vec![(x - dx, z - dz), (x + dx, z + dz)], false))
        }
        Shape::Time { .. } => None,
    }
}

fn set_handle(shape: &mut Shape, h: usize, p: (f64, f64), snap: bool) {
    match shape {
        Shape::Box { x0, z0, x1, z1 } => {
            if h == 0 {
                *x0 = snap_round(p.0, snap);
                *z0 = snap_round(p.1, snap);
            } else {
                *x1 = snap_round(p.0, snap) - 1.0;
                *z1 = snap_round(p.1, snap) - 1.0;
            }
        }
        Shape::Circle { cx, cz, radius } => {
            if h == 0 {
                *cx = snap_round(p.0, snap);
                *cz = snap_round(p.1, snap);
            } else {
                let r = ((p.0 - *cx).powi(2) + (p.1 - *cz).powi(2)).sqrt();
                *radius = if snap { (r / 16.0).round().max(1.0) * 16.0 } else { r.max(1.0) };
            }
        }
        Shape::Polygon { points } => {
            if let Some(pt) = points.get_mut(h) {
                *pt = (snap_round(p.0, snap), snap_round(p.1, snap));
            }
        }
        Shape::Split { x, z, angle_deg } => {
            if h == 0 {
                *x = snap_round(p.0, snap);
                *z = snap_round(p.1, snap);
            } else {
                let deg = (p.1 - *z).atan2(p.0 - *x).to_degrees();
                *angle_deg = if snap { (deg / 15.0).round() * 15.0 } else { deg };
            }
        }
        Shape::Time { .. } => {}
    }
}

impl PrunerState {
    pub fn new(dim: u8) -> Self {
        Self { ctl: MapController::new(dim, pruner_opts(dim)), ..Self::default() }
    }

    /// The Java InhabitedTime scan the Time layer needs (Bedrock has none).
    fn inhabited_scan(store: Option<Store>, dim: u8) -> Task<PrunerMsg> {
        match store {
            Some(store) if store.edition() == omt_world::Edition::Java => Task::perform(
                blocking(move || {
                    store
                        .prune_world()
                        .scan(dim, true)
                        .map(|v| v.into_iter().map(|c| ((c.x, c.z), c.inhabited.unwrap_or(0))).collect())
                        .map_err(|e| e.to_string())
                }),
                PrunerMsg::InhabitedReady,
            ),
            _ => Task::none(),
        }
    }

    /// (Re)load the current dimension — `rescan` after the world changed.
    fn reload(&mut self, sess: &WorldSession, rescan: bool) -> Task<PrunerMsg> {
        let store = sess.store.clone();
        let render = self.ctl.load(store.as_ref(), rescan).map(PrunerMsg::Tiles);
        Task::batch([render, Self::inhabited_scan(store, self.ctl.dim)])
    }

    pub fn load(&mut self, sess: &WorldSession) -> Task<PrunerMsg> {
        self.reload(sess, false)
    }

    /// Status-bar text: the last pruner action plus the map's progress and
    /// memory state.
    pub fn status_line(&self) -> String {
        match (self.status.is_empty(), self.ctl.status.is_empty()) {
            (true, _) => self.ctl.status.clone(),
            (false, true) => self.status.clone(),
            (false, false) => format!("{} — {}", self.status, self.ctl.status),
        }
    }

    fn recompute(&mut self) {
        let Some(map) = &self.ctl.map else { return };
        let chunks = map.source.chunks();
        let inh = self.inhabited.as_ref();
        self.red = chunks
            .iter()
            .filter(|&(x, z)| self.selection.selects(x, z, inh.and_then(|m| m.get(&(x, z)).copied())))
            .collect();
    }

    pub fn update(&mut self, sess: &mut WorldSession, msg: PrunerMsg) -> Task<PrunerMsg> {
        match msg {
            PrunerMsg::Map(ev) => return self.on_map(ev, sess),
            PrunerMsg::Tool(t) => {
                self.tool = t;
                self.polygon.clear();
            }
            PrunerMsg::Dim(d) => {
                if d != self.ctl.dim {
                    self.inhabited = None;
                    self.red.clear();
                    let store = sess.store.clone();
                    let t = self.ctl.set_dim(d, pruner_opts(d), store.as_ref()).map(PrunerMsg::Tiles);
                    return Task::batch([t, Self::inhabited_scan(store, d)]);
                }
            }
            PrunerMsg::MapUi(_) => {
                return self.ctl.refill(sess.store.as_ref()).map(PrunerMsg::Tiles);
            }
            PrunerMsg::Look(m) => {
                if self.look.apply(m) {
                    self.ctl.set_post(Some(self.look.style));
                }
            }
            PrunerMsg::Tiles(m) => {
                let was_loaded = self.ctl.map.is_some();
                let t = self.ctl.on_msg(m, sess.store.as_ref()).map(PrunerMsg::Tiles);
                if !was_loaded && self.ctl.map.is_some() {
                    self.recompute();
                }
                return t;
            }
            PrunerMsg::InhabitedReady(res) => match res {
                Ok(m) => {
                    self.inhabited = Some(m);
                    self.recompute();
                }
                Err(e) => self.status = format!("InhabitedTime scan failed: {e}"),
            },
            PrunerMsg::SelectLayer(i) => self.active = Some(i),
            PrunerMsg::ToggleInvert(i) => {
                if let Some(l) = self.selection.layers.get_mut(i) {
                    l.inverted = !l.inverted;
                }
                self.recompute();
            }
            PrunerMsg::ToggleEnable(i) => {
                if let Some(l) = self.selection.layers.get_mut(i) {
                    l.enabled = !l.enabled;
                }
                self.recompute();
            }
            PrunerMsg::DeleteLayer(i) => {
                if i < self.selection.layers.len() {
                    self.selection.layers.remove(i);
                    self.active = None;
                }
                self.recompute();
            }
            PrunerMsg::ClearLayers => {
                self.selection.layers.clear();
                self.active = None;
                self.recompute();
            }
            PrunerMsg::TimeOp(op) => self.time_op = op,
            PrunerMsg::TimeValue(v) => self.time_value = v.chars().filter(|c| c.is_ascii_digit()).collect(),
            PrunerMsg::AddTime => {
                let ticks = self.time_value.parse().unwrap_or(0);
                self.selection.layers.push(Layer::new(Shape::Time { op: self.time_op, ticks }));
                self.active = Some(self.selection.layers.len() - 1);
                self.recompute();
            }
            PrunerMsg::Prune => {
                if self.red.is_empty() {
                    self.status = "nothing selected".into();
                } else {
                    self.confirm = true;
                }
            }
            PrunerMsg::CancelPrune => self.confirm = false,
            PrunerMsg::ConfirmPrune => {
                self.confirm = false;
                let Some(store) = sess.store.clone() else { return Task::none() };
                self.busy = true;
                self.status = format!("Pruning {} chunks…", self.red.len());
                let (dim, sel) = (self.ctl.dim, self.selection.clone());
                return Task::perform(
                    blocking(move || {
                        let w = store.prune_world();
                        let plan = w.plan(dim, &sel).map_err(|e| e.to_string())?;
                        let r = w.execute(&plan).map_err(|e| e.to_string())?;
                        Ok(format!(
                            "Deleted {} chunks ({} entities/actors). Save to keep the result, Close to discard.",
                            r.chunks_deleted, r.actors_deleted
                        ))
                    }),
                    PrunerMsg::PruneDone,
                );
            }
            PrunerMsg::PruneDone(res) => {
                self.busy = false;
                match res {
                    Ok(s) => {
                        sess.modified = true;
                        self.status = s;
                        // The world changed: rebuild the chunk index + tiles.
                        return self.reload(sess, true);
                    }
                    Err(e) => self.status = format!("prune failed: {e}"),
                }
            }
            PrunerMsg::Optimize => {
                let Some(store) = sess.store.clone() else { return Task::none() };
                self.busy = true;
                self.status = "Optimizing database…".into();
                return Task::perform(
                    blocking(move || {
                        store.prune_world().optimize().map_err(|e| e.to_string())?;
                        Ok("Database optimized (deleted keys cleaned up).".to_string())
                    }),
                    PrunerMsg::OptimizeDone,
                );
            }
            PrunerMsg::OptimizeDone(res) => {
                self.busy = false;
                match res {
                    Ok(s) => {
                        sess.modified = true;
                        self.status = s;
                    }
                    Err(e) => self.status = format!("optimize failed: {e}"),
                }
            }
            PrunerMsg::FitView => {
                self.ctl.fit();
                return self.ctl.refill(sess.store.as_ref()).map(PrunerMsg::Tiles);
            }
            PrunerMsg::ToggleRender => self.render_open = !self.render_open,
            PrunerMsg::ToggleTime => self.time_open = !self.time_open,
        }
        Task::none()
    }

    fn on_map(&mut self, ev: MapEvent, sess: &WorldSession) -> Task<PrunerMsg> {
        let store = sess.store.as_ref();
        match ev {
            MapEvent::Pan { dx, dz } => {
                self.ctl.camera.cx += dx;
                self.ctl.camera.cz += dz;
                self.ctl.refill(store).map(PrunerMsg::Tiles)
            }
            MapEvent::Zoom { factor, at } => {
                self.ctl.camera.zoom_at(factor, at);
                self.ctl.refill(store).map(PrunerMsg::Tiles)
            }
            MapEvent::Resize(size) => self.ctl.on_viewport(size, store).map(PrunerMsg::Tiles),
            MapEvent::Hover { at } => {
                self.hover = Some(at);
                Task::none()
            }
            MapEvent::Press { at, right, ctrl } => {
                let snap = !ctrl;
                if right {
                    if self.tool == Tool::Polygon && self.polygon.len() >= 3 {
                        let points = std::mem::take(&mut self.polygon);
                        self.push_layer(Shape::Polygon { points });
                    }
                    return Task::none();
                }
                // Grab a control point of the active layer first.
                if let Some(li) = self.active
                    && let Some(layer) = self.selection.layers.get(li)
                {
                    let tol = 8.0 / self.ctl.camera.zoom as f64;
                    if let Some(h) = handles(&layer.shape)
                        .iter()
                        .position(|p| (p.0 - at.0).abs() <= tol && (p.1 - at.1).abs() <= tol)
                    {
                        self.drag = Some(Drag::Handle { layer: li, handle: h });
                        return Task::none();
                    }
                }
                match self.tool {
                    Tool::Polygon => {
                        self.polygon.push((snap_round(at.0, snap), snap_round(at.1, snap)));
                        Task::none()
                    }
                    _ => {
                        let anchor = match self.tool {
                            Tool::Circle => (snap_floor(at.0, snap) + if snap { 8.0 } else { 0.0 }, snap_floor(at.1, snap) + if snap { 8.0 } else { 0.0 }),
                            Tool::Split => (snap_round(at.0, snap), snap_round(at.1, snap)),
                            _ => (snap_floor(at.0, snap), snap_floor(at.1, snap)),
                        };
                        let shape = match self.tool {
                            Tool::Box => Shape::Box { x0: anchor.0, z0: anchor.1, x1: anchor.0 + if snap { 15.0 } else { 0.0 }, z1: anchor.1 + if snap { 15.0 } else { 0.0 } },
                            Tool::Circle => Shape::Circle { cx: anchor.0, cz: anchor.1, radius: if snap { 16.0 } else { 1.0 } },
                            _ => Shape::Split { x: anchor.0, z: anchor.1, angle_deg: 0.0 },
                        };
                        self.push_layer(shape);
                        self.drag = Some(Drag::New(anchor));
                        Task::none()
                    }
                }
            }
            MapEvent::Drag { at, ctrl } => {
                self.drag_to(at, !ctrl);
                Task::none()
            }
            MapEvent::Release { at, ctrl } => {
                self.drag_to(at, !ctrl);
                self.drag = None;
                Task::none()
            }
        }
    }

    fn push_layer(&mut self, shape: Shape) {
        self.selection.layers.push(Layer::new(shape));
        self.active = Some(self.selection.layers.len() - 1);
        self.recompute();
    }

    fn drag_to(&mut self, at: (f64, f64), snap: bool) {
        match self.drag {
            Some(Drag::New(anchor)) => {
                let Some(li) = self.active else { return };
                let Some(layer) = self.selection.layers.get_mut(li) else { return };
                match &mut layer.shape {
                    Shape::Box { x0, z0, x1, z1 } => {
                        let (ex, ez) = (snap_floor(at.0, snap), snap_floor(at.1, snap));
                        let pad = if snap { 15.0 } else { 0.0 };
                        *x0 = anchor.0.min(ex);
                        *z0 = anchor.1.min(ez);
                        *x1 = anchor.0.max(ex) + pad;
                        *z1 = anchor.1.max(ez) + pad;
                    }
                    shape @ Shape::Circle { .. } => set_handle(shape, 1, at, snap),
                    shape @ Shape::Split { .. } => set_handle(shape, 1, at, snap),
                    _ => {}
                }
                self.recompute();
            }
            Some(Drag::Handle { layer, handle }) => {
                if let Some(l) = self.selection.layers.get_mut(layer) {
                    set_handle(&mut l.shape, handle, at, snap);
                }
                self.recompute();
            }
            None => {}
        }
    }

    /// Use a saved map look (startup).
    pub fn set_look(&mut self, look: crate::maplook::MapLook) {
        self.look = look;
        self.ctl.set_post(Some(look.style));
    }

    fn overlay(&self, spawn: Option<(i32, i32)>) -> Overlay {
        let mut outlines = Vec::new();
        for (i, l) in self.selection.layers.iter().enumerate() {
            if let Some((points, closed)) = outline(&l.shape) {
                let active = self.active == Some(i);
                outlines.push(Outline { points, closed, active, handles: if active { handles(&l.shape) } else { Vec::new() } });
            }
        }
        if !self.polygon.is_empty() {
            outlines.push(Outline { points: self.polygon.clone(), closed: false, active: true, handles: self.polygon.clone() });
        }
        let caption = match self.hover {
            Some((x, z)) => {
                let (bx, bz) = (x.floor() as i32, z.floor() as i32);
                let ident = self
                    .ctl
                    .map
                    .as_ref()
                    .and_then(|m| m.identify(bx, bz))
                    .map(|(n, y, approx)| format!(" · {}{n} @ Y {y}", if approx { "≈ " } else { "" }))
                    .unwrap_or_default();
                format!("Block {bx}, {bz} · Chunk {}, {}{ident} · {} selected", bx.div_euclid(16), bz.div_euclid(16), self.red.len())
            }
            None => format!("{} selected", self.red.len()),
        };
        let look = &self.look;
        let markers = match (look.spawn && self.ctl.dim == 0, spawn) {
            (true, Some((x, z))) => vec![crate::mapview::Marker { x: x as f64 + 0.5, z: z as f64 + 0.5, label: "Spawn".into(), icon: None, color: iced::Color::WHITE }],
            _ => Vec::new(),
        };
        Overlay { red: self.red.clone(), outlines, marker: None, caption, markers, chunk_grid: look.chunk_grid, region_grid: look.region_grid, hud: look.hud, ruler: look.ruler, cursor: self.hover, ..Default::default() }
    }

    pub fn view<'a>(&'a self, sess: &'a WorldSession) -> Element<'a, PrunerMsg> {
        let bedrock = sess.edition() == omt_world::Edition::Bedrock;
        let time_row = row![
            pick_list([TimeOp::Greater, TimeOp::Equal, TimeOp::Less].map(OpLabel).to_vec(), Some(OpLabel(self.time_op)), |o: OpLabel| PrunerMsg::TimeOp(o.0)).width(64),
            text_input("ticks", &self.time_value).on_input(PrunerMsg::TimeValue).width(Length::Fill),
            button(text("Add").size(12)).on_press_maybe((!bedrock).then_some(PrunerMsg::AddTime)).style(style::button_style(Btn::Standard)),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center);
        let time_note = if bedrock {
            "Time: Java & Legacy Console only — Bedrock stores no InhabitedTime."
        } else {
            "Time: select chunks by InhabitedTime (20 ticks = 1 second)."
        };
        let optimize = button(text("⚙ Optimize").size(12))
            .on_press_maybe((bedrock && !self.busy).then_some(PrunerMsg::Optimize))
            .style(style::button_style(Btn::Standard))
            .padding([6, 8]);

        let layers = self.selection.layers.iter().enumerate().fold(column![].spacing(4), |c, (i, l)| {
            let desc = layer_desc(&l.shape);
            let head = button(column![text(l.shape.kind()).size(13), text(desc).size(11).color(style::muted())])
                .on_press(PrunerMsg::SelectLayer(i))
                .width(Length::Fill)
                .style(style::button_style(if self.active == Some(i) { Btn::Primary } else { Btn::Ghost }));
            c.push(
                container(column![
                    head,
                    row![
                        toggler(l.inverted).label("Inverted").size(14).text_size(11).on_toggle(move |_| PrunerMsg::ToggleInvert(i)),
                        toggler(l.enabled).label("On").size(14).text_size(11).on_toggle(move |_| PrunerMsg::ToggleEnable(i)),
                        button(text("✕").size(11)).on_press(PrunerMsg::DeleteLayer(i)).style(style::button_style(Btn::Ghost)),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                ])
                .padding(6)
                .style(style::card()),
            )
        });
        let total = self.ctl.map.as_ref().map_or(0, MapData::chunk_total);
        let prune_area: Element<'_, PrunerMsg> = if self.confirm {
            column![
                text(format!("Delete {} of {} chunks (with their entities and POI)?", self.red.len(), total)).size(12).color(style::warn()),
                row![
                    button(text("Prune now").size(12)).on_press(PrunerMsg::ConfirmPrune).style(style::button_style(Btn::Danger)),
                    button(text("Cancel").size(12)).on_press(PrunerMsg::CancelPrune).style(style::button_style(Btn::Standard)),
                ]
                .spacing(6),
            ]
            .spacing(6)
            .into()
        } else {
            button(text(format!("✂ Prune {} chunks", self.red.len())).size(13))
                .on_press_maybe((!self.busy && !self.red.is_empty()).then_some(PrunerMsg::Prune))
                .width(Length::Fill)
                .padding([8, 10])
                .style(style::button_style(Btn::Danger))
                .into()
        };
        let mut sidebar = column![text(self.tool.hint()).size(11).color(style::muted())].spacing(8);
        if self.time_open {
            sidebar = sidebar
                .push(time_row)
                .push(text(time_note).size(11).color(style::muted()))
                .push(row![optimize, text("Optimize: run a database compaction cycle (Bedrock only).").size(11).color(style::muted())].spacing(6).align_y(iced::Alignment::Center));
        }
        sidebar = sidebar
            .push(iced::widget::rule::horizontal(1.0))
            .push(
                row![
                    text(format!("Layers ({})", self.selection.layers.len())).size(14),
                    iced::widget::space::horizontal(),
                    button(text("Clear").size(11)).on_press(PrunerMsg::ClearLayers).style(style::button_style(Btn::Ghost)),
                ]
                .align_y(iced::Alignment::Center),
            )
            .push(scrollable(layers).height(Length::Fill))
            .push(text(format!("{} of {} chunks selected", self.red.len(), total)).size(12))
            .push(prune_area);
        let sidebar = sidebar.padding(10).width(268);

        let viewer: Element<'_, PrunerMsg> = match &self.ctl.map {
            Some(map) => crate::mapview::element(MapView { map, camera: self.ctl.camera, overlay: self.overlay(sess.summary.spawn), on_event: Box::new(PrunerMsg::Map) }),
            None => container(text(if self.ctl.loading { self.ctl.status.as_str() } else { "No map" }).color(style::muted())).center(Length::Fill).into(),
        };
        let nav = container(self.nav()).padding([6, 8]).width(Length::Fill).style(style::topbar());
        let mut over_map = column![nav, viewer].width(Length::Fill);
        if self.render_open {
            let mut row2 = row![text("Sun direction").size(12).color(style::muted())].spacing(6);
            if !matches!(self.look.style.style, omt_render::style::MapStyle::Heightmap) {
                row2 = row2.push(slider(0.0..=359.0, self.look.style.azimuth, |v| PrunerMsg::Look(crate::maplook::LookMsg::Azimuth(v))).step(5.0f32).width(110));
            }
            over_map = over_map.push(
                container(row![crate::mapdata::controls(PrunerMsg::MapUi), space::horizontal(), row2].spacing(12).padding([4, 8]).align_y(iced::Alignment::Center))
                    .padding([0, 8])
                    .width(Length::Fill)
                    .style(style::panel()),
            );
        }
        row![
            container(sidebar).height(Length::Fill).style(style::panel()),
            row![self.tool_rail(), over_map].width(Length::Fill),
        ]
        .height(Length::Fill)
        .into()
    }

    /// The map's container nav: dimension, map style, grids, rulers, HUD
    /// toggles, the render disclosure and Fit — one compact row.
    fn nav(&self) -> Element<'_, PrunerMsg> {
        use iced::widget::toggler;
        let dims = pick_list(Dimension::ALL.map(|d| DimChoice(d.id())), Some(DimChoice(self.ctl.dim)), |d: DimChoice| PrunerMsg::Dim(d.0)).text_size(12).width(100);
        let styles = pick_list(MapStyle::ALL.to_vec(), Some(self.look.style.style), |s| PrunerMsg::Look(crate::maplook::LookMsg::Style(s))).text_size(12).width(116);
        let grids = pick_list(crate::maplook::GridChoice::ALL.to_vec(), Some(crate::maplook::GridChoice(self.look.chunk_grid)), |g| PrunerMsg::Look(crate::maplook::LookMsg::ChunkGrid(g))).text_size(12).width(92);
        let rulers = pick_list(crate::mapview_overlay::Ruler::ALL.to_vec(), Some(self.look.ruler), |r| PrunerMsg::Look(crate::maplook::LookMsg::Ruler(r))).text_size(12).width(110);
        row![
            dims,
            styles,
            grids,
            rulers,
            toggler(self.look.region_grid).label("Regions").size(13).text_size(11).width(78).on_toggle(|v| PrunerMsg::Look(crate::maplook::LookMsg::RegionGrid(v))),
            toggler(self.look.hud).label("Scale").size(13).text_size(11).width(66).on_toggle(|v| PrunerMsg::Look(crate::maplook::LookMsg::Hud(v))),
            toggler(self.look.spawn).label("Spawn").size(13).text_size(11).width(72).on_toggle(|v| PrunerMsg::Look(crate::maplook::LookMsg::Spawn(v))),
            iced::widget::space::horizontal(),
            button(text("Sun").size(11)).padding([5, 7]).on_press(PrunerMsg::ToggleRender).style(style::button_style(if self.render_open { Btn::ChipOn } else { Btn::Chip })),
            button(text("Fit").size(12)).padding([5, 7]).on_press(PrunerMsg::FitView).style(style::button_style(Btn::Standard)),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .into()
    }

    /// Photoshop-style tool rail docked left of the map (shared geometry
    /// with the Map Viewer's rail — `maprail`).
    fn tool_rail(&self) -> Element<'_, PrunerMsg> {
        let right = tooltip::Position::Right;
        let mut col: Vec<Element<'_, PrunerMsg>> = Tool::ALL
            .iter()
            .map(|&t| crate::maprail::button(crate::maprail::glyph(t.rail_glyph()), t.hint(), self.tool == t, PrunerMsg::Tool(t), right))
            .collect();
        col.push(crate::maprail::separator());
        col.push(crate::maprail::button(crate::maprail::glyph("⏱"), "Time: select chunks by InhabitedTime", self.time_open, PrunerMsg::ToggleTime, right));
        crate::maprail::strip(col)
    }
}

/// A dimension value for the nav pick list (shows its name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DimChoice(u8);

impl std::fmt::Display for DimChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(Dimension::from_id(self.0).map_or("?", Dimension::name))
    }
}

/// Dimension switcher chips (Overworld / Nether / The End).
pub fn dim_switcher<'a, M: Clone + 'a>(current: u8, on: impl Fn(u8) -> M + 'a) -> Element<'a, M> {
    omt_world::Dimension::ALL
        .iter()
        .fold(row![].spacing(4), |r, d| {
            r.push(
                button(text(d.name()).size(12))
                    .on_press(on(d.id()))
                    .padding([4, 10])
                    .style(style::button_style(if d.id() == current { Btn::Primary } else { Btn::Standard })),
            )
        })
        .into()
}

fn layer_desc(s: &Shape) -> String {
    match s {
        Shape::Box { x0, z0, x1, z1 } => format!("{x0:.0}, {z0:.0} → {x1:.0}, {z1:.0}"),
        Shape::Circle { cx, cz, radius } => format!("center {cx:.0}, {cz:.0} · r {radius:.0}"),
        Shape::Polygon { points } => format!("{} points", points.len()),
        Shape::Split { x, z, angle_deg } => format!("{x:.0}, {z:.0} · {angle_deg:.0}°"),
        Shape::Time { op, ticks } => format!("InhabitedTime {} {ticks} ({:.1} s)", op.symbol(), *ticks as f64 / 20.0),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpLabel(TimeOp);

impl std::fmt::Display for OpLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.symbol())
    }
}
