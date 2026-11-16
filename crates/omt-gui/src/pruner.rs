//! Pruner app (UMT "Pruner"): selection controls on the left, the 2D world
//! viewer on the right. Tools: Box · Circle · Polygon · Split · Time ·
//! Optimize. Each drawn shape becomes a layer; layers union; selected
//! chunks show red; Prune deletes them from the workspace copy (Save writes
//! back, Close discards).

use std::collections::HashMap;

use iced::widget::{button, canvas, column, container, pick_list, row, scrollable, text, text_input, toggler};
use iced::{Element, Length, Size, Task};
use omt_prune::{Layer, Selection, Shape, TimeOp};

use crate::mapdata::{MapBuild, MapData};
use crate::mapview::{Camera, MapEvent, MapView, Outline, Overlay};
use crate::session::{WorldSession, blocking};
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
    fn label(self) -> &'static str {
        match self {
            Self::Box => "▭ Box",
            Self::Circle => "◯ Circle",
            Self::Polygon => "⬠ Polygon",
            Self::Split => "⟋ Split",
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
    MapReady(u8, Result<std::sync::Arc<std::sync::Mutex<Option<MapBuild>>>, String>),
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
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    /// Drawing a new shape from an anchor point.
    New((f64, f64)),
    /// Moving control point `handle` of layer `layer`.
    Handle { layer: usize, handle: usize },
}

pub struct PrunerState {
    pub dim: u8,
    pub camera: Camera,
    pub map: Option<MapData>,
    pub loading: bool,
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
    pub status: String,
    hover: Option<(f64, f64)>,
    fitted: bool,
}

impl Default for PrunerState {
    fn default() -> Self {
        Self {
            dim: 0,
            camera: Camera::default(),
            map: None,
            loading: false,
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
            status: String::new(),
            hover: None,
            fitted: false,
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
        Self { dim, ..Self::default() }
    }

    /// Kick off (re)rendering the current dimension (and the Java
    /// InhabitedTime scan the Time layer needs).
    pub fn load(&mut self, sess: &WorldSession) -> Task<PrunerMsg> {
        let Some(store) = sess.store.clone() else { return Task::none() };
        self.loading = true;
        self.status = "Rendering map…".into();
        let dim = self.dim;
        let opts = omt_render::RenderOptions::for_dimension(dim);
        let s2 = store.clone();
        let render = Task::perform(blocking(move || crate::mapdata::build(s2, dim, opts)), move |r| {
            PrunerMsg::MapReady(dim, r.map(|b| std::sync::Arc::new(std::sync::Mutex::new(Some(b)))))
        });
        let scan = if store.edition() == omt_world::Edition::Java {
            Task::perform(
                blocking(move || {
                    store
                        .prune_world()
                        .scan(dim, true)
                        .map(|v| v.into_iter().map(|c| ((c.x, c.z), c.inhabited.unwrap_or(0))).collect())
                        .map_err(|e| e.to_string())
                }),
                PrunerMsg::InhabitedReady,
            )
        } else {
            Task::none()
        };
        Task::batch([render, scan])
    }

    fn recompute(&mut self) {
        let Some(map) = &self.map else { return };
        let inh = self.inhabited.as_ref();
        self.red = map
            .chunks
            .iter()
            .copied()
            .filter(|&(x, z)| self.selection.selects(x, z, inh.and_then(|m| m.get(&(x, z)).copied())))
            .collect();
    }

    pub fn update(&mut self, sess: &mut WorldSession, msg: PrunerMsg) -> Task<PrunerMsg> {
        match msg {
            PrunerMsg::Map(ev) => self.on_map(ev),
            PrunerMsg::Tool(t) => {
                self.tool = t;
                self.polygon.clear();
            }
            PrunerMsg::Dim(d) => {
                if d != self.dim {
                    self.dim = d;
                    self.map = None;
                    self.inhabited = None;
                    self.fitted = false;
                    return self.load(sess);
                }
            }
            PrunerMsg::MapReady(dim, res) => {
                if dim != self.dim {
                    return Task::none();
                }
                self.loading = false;
                match res {
                    Ok(slot) => {
                        if let Some(build) = slot.lock().ok().and_then(|mut g| g.take()) {
                            let data = MapData::from_build(build);
                            self.status = format!(
                                "{} chunks rendered in {:.1}s{}",
                                data.chunks.len(),
                                data.millis as f32 / 1000.0,
                                if data.errors > 0 { format!(" ({} unreadable)", data.errors) } else { String::new() }
                            );
                            if !self.fitted && let Some(b) = data.chunk_bounds() {
                                self.camera = Camera::fit(b, Size::new(900.0, 640.0));
                                self.fitted = true;
                            }
                            self.map = Some(data);
                            self.recompute();
                        }
                    }
                    Err(e) => self.status = format!("render failed: {e}"),
                }
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
                let (dim, sel) = (self.dim, self.selection.clone());
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
                        let status = self.status.clone();
                        let t = self.load(sess);
                        self.status = status;
                        return t;
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
                if let Some(b) = self.map.as_ref().and_then(MapData::chunk_bounds) {
                    self.camera = Camera::fit(b, Size::new(900.0, 640.0));
                }
            }
        }
        Task::none()
    }

    fn on_map(&mut self, ev: MapEvent) {
        match ev {
            MapEvent::Pan { dx, dz } => {
                self.camera.cx += dx;
                self.camera.cz += dz;
            }
            MapEvent::Zoom { factor, at } => self.camera.zoom_at(factor, at),
            MapEvent::Hover { at } => self.hover = Some(at),
            MapEvent::Press { at, right, ctrl } => {
                let snap = !ctrl;
                if right {
                    if self.tool == Tool::Polygon && self.polygon.len() >= 3 {
                        let points = std::mem::take(&mut self.polygon);
                        self.push_layer(Shape::Polygon { points });
                    }
                    return;
                }
                // Grab a control point of the active layer first.
                if let Some(li) = self.active
                    && let Some(layer) = self.selection.layers.get(li)
                {
                    let tol = 8.0 / self.camera.zoom as f64;
                    if let Some(h) = handles(&layer.shape)
                        .iter()
                        .position(|p| (p.0 - at.0).abs() <= tol && (p.1 - at.1).abs() <= tol)
                    {
                        self.drag = Some(Drag::Handle { layer: li, handle: h });
                        return;
                    }
                }
                match self.tool {
                    Tool::Polygon => self.polygon.push((snap_round(at.0, snap), snap_round(at.1, snap))),
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
                    }
                }
            }
            MapEvent::Drag { at, ctrl } => self.drag_to(at, !ctrl),
            MapEvent::Release { at, ctrl } => {
                self.drag_to(at, !ctrl);
                self.drag = None;
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

    fn overlay(&self) -> Overlay {
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
                let ident = self.map.as_ref().and_then(|m| m.identify(bx, bz)).map(|(n, y)| format!(" · {n} @ Y {y}")).unwrap_or_default();
                format!("Block {bx}, {bz} · Chunk {}, {}{ident} · {} selected", bx.div_euclid(16), bz.div_euclid(16), self.red.len())
            }
            None => format!("{} selected", self.red.len()),
        };
        Overlay { red: self.red.clone(), outlines, marker: None, caption }
    }

    pub fn view<'a>(&'a self, sess: &'a WorldSession) -> Element<'a, PrunerMsg> {
        let bedrock = sess.edition() == omt_world::Edition::Bedrock;
        let tools = Tool::ALL.iter().fold(row![].spacing(4), |r, &t| {
            r.push(
                button(text(t.label()).size(12))
                    .on_press(PrunerMsg::Tool(t))
                    .padding([6, 8])
                    .style(style::button_style(if self.tool == t { Btn::Primary } else { Btn::Standard })),
            )
        });
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
            let head = button(column![text(l.shape.kind()).size(13), text(desc).size(11).color(style::MUTED)])
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
        let total = self.map.as_ref().map_or(0, |m| m.chunks.len());
        let prune_area: Element<'_, PrunerMsg> = if self.confirm {
            column![
                text(format!("Delete {} of {} chunks (with their entities and POI)?", self.red.len(), total)).size(12).color(style::WARN),
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
        let sidebar = column![
            text("Tools").size(14),
            tools,
            text(self.tool.hint()).size(11).color(style::MUTED),
            text("Hold Ctrl to disable chunk snapping. Wheel zooms, right/middle-drag pans.").size(11).color(style::HINT),
            time_row,
            text(time_note).size(11).color(style::MUTED),
            row![optimize, text("Optimize: run a database compaction cycle (Bedrock only).").size(11).color(style::MUTED)].spacing(6).align_y(iced::Alignment::Center),
            iced::widget::rule::horizontal(1.0),
            row![
                text(format!("Layers ({})", self.selection.layers.len())).size(14),
                iced::widget::space::horizontal(),
                button(text("Clear").size(11)).on_press(PrunerMsg::ClearLayers).style(style::button_style(Btn::Ghost)),
            ]
            .align_y(iced::Alignment::Center),
            scrollable(layers).height(Length::Fill),
            text(format!("{} of {} chunks selected", self.red.len(), total)).size(12),
            prune_area,
        ]
        .spacing(8)
        .padding(10)
        .width(300);

        let dims = dim_switcher(self.dim, PrunerMsg::Dim);
        let viewer: Element<'_, PrunerMsg> = match &self.map {
            Some(map) => canvas(MapView { map, camera: self.camera, overlay: self.overlay(), on_event: Box::new(PrunerMsg::Map) })
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => container(text(if self.loading { "Rendering map…" } else { "No map" }).color(style::MUTED)).center(Length::Fill).into(),
        };
        let top = row![dims, iced::widget::space::horizontal(), button(text("Fit").size(12)).on_press(PrunerMsg::FitView).style(style::button_style(Btn::Ghost))]
            .spacing(6)
            .padding([4, 8])
            .align_y(iced::Alignment::Center);
        row![
            container(sidebar).height(Length::Fill).style(style::panel()),
            column![top, viewer].width(Length::Fill),
        ]
        .height(Length::Fill)
        .into()
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
