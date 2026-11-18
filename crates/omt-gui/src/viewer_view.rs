//! Map Viewer UI: a control sidebar (dimension, style, sun lighting,
//! layers, render, navigation, bookmarks, export) beside the map canvas
//! with floating HUD chips.

use iced::widget::{button, column, container, row, scrollable, slider, space, stack, text, text_input, toggler};
use iced::{Alignment, Color, Element, Length};
use omt_art::{Glyph, Icon};
use omt_render::style::MapStyle;

use crate::assets;
use crate::mapview::{MapView, Marker, Overlay};
use crate::style::{self, Btn};
use crate::viewer::{Bookmark, Layer, ViewerMsg, ViewerState};

fn section<'a>(icon: Icon, title: &'a str, body: Element<'a, ViewerMsg>) -> Element<'a, ViewerMsg> {
    container(
        column![
            row![assets::icon(icon, 16.0), text(title).size(13).font(style::BOLD)].spacing(8).align_y(Alignment::Center),
            body,
        ]
        .spacing(10),
    )
    .padding(12)
    .style(style::card_flat())
    .into()
}

fn labeled<'a>(label: &'a str, value: String, control: Element<'a, ViewerMsg>) -> Element<'a, ViewerMsg> {
    column![row![text(label).size(12).color(style::muted()), space::horizontal(), text(value).size(12)], control].spacing(4).into()
}

fn switch<'a>(label: &'a str, on: bool, f: impl Fn(bool) -> ViewerMsg + 'a) -> Element<'a, ViewerMsg> {
    row![text(label).size(12).width(Length::Fill), toggler(on).size(16).on_toggle(f)].align_y(Alignment::Center).into()
}

/// Segmented button row.
fn segmented<'a, T: Copy + PartialEq + 'a>(items: &[(T, &'a str)], current: T, f: impl Fn(T) -> ViewerMsg + 'a) -> Element<'a, ViewerMsg> {
    items
        .iter()
        .fold(row![].spacing(4), |r, &(v, label)| {
            r.push(button(text(label).size(12).center()).width(Length::Fill).padding([5, 4]).on_press(f(v)).style(style::button_style(if v == current { Btn::ChipOn } else { Btn::Chip })))
        })
        .into()
}

impl ViewerState {
    fn sidebar<'a>(&'a self, bookmarks: &'a [Bookmark]) -> Element<'a, ViewerMsg> {
        let p = &self.prefs;
        let s = &p.style;
        let dims = segmented(&[(0u8, "Overworld"), (1, "Nether"), (2, "End")], self.ctl.dim, ViewerMsg::Dim);
        let styles = MapStyle::ALL.iter().fold(column![].spacing(4), |c, &st| {
            let on = st == s.style;
            let desc = match st {
                MapStyle::Classic => "Blocky relief, like the classic tools",
                MapStyle::Shaded => "Sun-lit terrain with soft shadows",
                MapStyle::Topographic => "Elevation colors + contour lines",
                MapStyle::Heightmap => "Grayscale elevation",
                MapStyle::Night => "Moonlight + glowing torches & lava",
                MapStyle::Biomes => "Biome map with relief",
            };
            c.push(
                button(column![text(st.name()).size(13), text(desc).size(11).color(style::muted())].spacing(1))
                    .width(Length::Fill)
                    .padding([6, 10])
                    .on_press(ViewerMsg::Style(st))
                    .style(style::button_style(if on { Btn::ChipOn } else { Btn::Nav })),
            )
        });
        let mut light = column![
            labeled("Sun direction", format!("{:.0}°", s.azimuth), slider(0.0..=359.0, s.azimuth, ViewerMsg::Azimuth).step(1.0f32).into()),
            labeled("Sun height", format!("{:.0}°", s.altitude), slider(5.0..=90.0, s.altitude, ViewerMsg::Altitude).step(1.0f32).into()),
            labeled("Relief", format!("{:.0}%", s.strength * 100.0), slider(0.0..=1.5, s.strength, ViewerMsg::Strength).step(0.05f32).into()),
        ]
        .spacing(8);
        if s.style == MapStyle::Topographic {
            light = light.push(labeled("Contour every", format!("{} blocks", s.contour), slider(0..=64, s.contour, ViewerMsg::Contour).step(4).into()));
        }
        if s.style == MapStyle::Biomes {
            let legend = self.legend.iter().take(14).fold(column![].spacing(3), |c, (name, n)| {
                let rgb = omt_render::style::biome_color(name);
                let col = Color::from_rgb8(rgb[0], rgb[1], rgb[2]);
                let total: usize = self.legend.iter().map(|x| x.1).sum::<usize>().max(1);
                c.push(
                    row![
                        iced::widget::container(space::horizontal().width(12)).height(12).style(move |_| iced::widget::container::Style { background: Some(iced::Background::Color(col)), border: iced::border::rounded(2), ..Default::default() }),
                        text(name.trim_start_matches("minecraft:").replace('_', " ")).size(11).width(Length::Fill),
                        text(format!("{:.0}%", *n as f32 * 100.0 / total as f32)).size(11).color(style::hint()),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
            });
            light = light.push(text("Biomes in view").size(12).color(style::muted())).push(legend);
        }
        if s.style == MapStyle::Night {
            light = light.push(labeled("Moonlight", format!("{:.0}%", s.night_ambient * 100.0), slider(0.0..=0.6, s.night_ambient, ViewerMsg::Ambient).step(0.02f32).into()));
        }
        let grid = segmented(&[(None, "Auto"), (Some(true), "On"), (Some(false), "Off")], p.chunk_grid, ViewerMsg::ChunkGrid);
        let ent_label = if self.entities.is_empty() { "Entities (zoom in)".to_string() } else { format!("Entities ({} in view)", self.entities.len()) };
        let slime_note = if !self.bedrock && self.seed.is_none() { " (seed unknown)" } else { "" };
        let layers = column![
            labeled("Chunk grid", String::new(), grid),
            labeled("Rulers", String::new(), segmented(&crate::mapview_overlay::Ruler::ALL.map(|r| (r, r.name())), p.ruler, ViewerMsg::Ruler)),
            switch("Region grid (32×32 chunks)", p.region_grid, |v| ViewerMsg::Layer(Layer::RegionGrid, v)),
            switch(if slime_note.is_empty() { "Slime chunks" } else { "Slime chunks (seed unknown)" }, p.slime, |v| ViewerMsg::Layer(Layer::Slime, v)),
            switch("World spawn", p.spawn, |v| ViewerMsg::Layer(Layer::Spawn, v)),
            switch("Players", p.players, |v| ViewerMsg::Layer(Layer::Players, v)),
            switch("Bookmarks", p.bookmarks, |v| ViewerMsg::Layer(Layer::Bookmarks, v)),
            switch("Scale bar & compass", p.hud, |v| ViewerMsg::Layer(Layer::Hud, v)),
            row![text(ent_label).size(12).width(Length::Fill), toggler(p.entities).size(16).on_toggle(|v| ViewerMsg::Layer(Layer::Entities, v))].align_y(Alignment::Center),
            switch(if crate::textures::current().is_some() { "Block textures when zoomed in" } else { "Block textures (no pack — see Settings)" }, p.textured, |v| ViewerMsg::Layer(Layer::Textures, v)),
        ]
        .spacing(8);
        let mut render = column![switch("Underground (caves)", self.cave, ViewerMsg::Cave), switch("Slicer", self.slicer.is_some(), ViewerMsg::Slicer), switch("Biome colors", p.biome_tint, |v| ViewerMsg::Layer(Layer::BiomeTint, v)),].spacing(8);
        if let Some(y) = self.slicer {
            let (lo, hi) = if self.ctl.dim == 1 { (0, 127) } else { (-64, 319) };
            render = render.push(labeled("Hide blocks above Y", y.to_string(), slider(lo..=hi, y, ViewerMsg::SlicerY).into()));
        }
        render = render.push(crate::mapdata::controls(ViewerMsg::MapUi));
        let nav = column![
            row![
                text_input("X", &self.goto.0).on_input(ViewerMsg::GotoX).on_submit(ViewerMsg::Goto).size(12).style(style::input()),
                text_input("Z", &self.goto.1).on_input(ViewerMsg::GotoZ).on_submit(ViewerMsg::Goto).size(12).style(style::input()),
                button(assets::glyph(Glyph::Target, style::t().on_accent, 14.0)).padding(6).on_press(ViewerMsg::Goto).style(style::button_style(Btn::Primary)),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            row![
                button(text("Fit world").size(12)).on_press(ViewerMsg::Fit).style(style::button_style(Btn::Standard)),
                button(assets::glyph(Glyph::Plus, style::fg(), 14.0)).padding(6).on_press(ViewerMsg::Zoom(2.0)).style(style::button_style(Btn::Standard)),
                button(text("−").size(14)).padding([2, 10]).on_press(ViewerMsg::Zoom(0.5)).style(style::button_style(Btn::Standard)),
            ]
            .spacing(6),
            text("Tip: click the map to copy coordinates here.").size(11).color(style::hint()),
        ]
        .spacing(8);
        let mine = self.my_bookmarks(bookmarks);
        let list = mine.iter().enumerate().fold(column![].spacing(4), |c, (i, (_, b))| {
            let dim = ["", " · Nether", " · End"].get(b.dim as usize).copied().unwrap_or("");
            c.push(
                row![
                    button(column![text(b.name.clone()).size(12), text(format!("{}, {}{dim}", b.x, b.z)).size(10).color(style::muted())]).width(Length::Fill).padding([4, 8]).on_press(ViewerMsg::GoBookmark(i)).style(style::button_style(Btn::Nav)),
                    button(assets::glyph(Glyph::Close, style::hint(), 12.0)).padding(6).on_press(ViewerMsg::RemoveBookmark(i)).style(style::button_style(Btn::Ghost)),
                ]
                .align_y(Alignment::Center),
            )
        });
        let marks = column![
            row![
                text_input("Name this place…", &self.bookmark_name).on_input(ViewerMsg::BookmarkName).on_submit(ViewerMsg::AddBookmark).size(12).style(style::input()),
                button(text("Add").size(12)).on_press(ViewerMsg::AddBookmark).style(style::button_style(Btn::Primary)),
            ]
            .spacing(6),
            list,
        ]
        .spacing(8);
        let export = button(row![assets::glyph(Glyph::Export, style::fg(), 14.0), text("Export view as PNG…").size(12)].spacing(8).align_y(Alignment::Center))
            .width(Length::Fill)
            .on_press(ViewerMsg::Export)
            .style(style::button_style(Btn::Standard));
        let mode = segmented(&[(false, "2D map"), (true, "3D terrain")], self.v3.on, ViewerMsg::Mode3D);
        let mut view_body = column![mode].spacing(8);
        if self.v3.on {
            let c = &self.v3.cam;
            {
                let has = if self.v3.blocks { self.v3.bmesh.is_some() } else { self.v3.mesh.is_some() };
                let gpu_label = match (has.then_some(()), self.v3.gpu) {
                    (Some(_), true) => format!("GPU · {}", omt_gpu::gpu().map(|g| g.adapter_name.clone()).unwrap_or_default()),
                    (None, true) => "GPU (not available — CPU)".to_string(),
                    _ => "GPU (off — CPU ray-casting)".to_string(),
                };
                view_body = view_body.push(row![text(gpu_label).size(12).width(Length::Fill), toggler(self.v3.gpu).size(16).on_toggle(ViewerMsg::Gpu3)].align_y(Alignment::Center));
            }
            view_body = view_body.push(labeled(
                "Detail",
                String::new(),
                segmented(&[(false, "Terrain (fast)"), (true, "Blocks (textured)")], self.v3.blocks, ViewerMsg::Detail3),
            ));
            let quality = [(0.5f32, "50%"), (0.75, "75%"), (1.0, "100%"), (1.5, "150%"), (2.0, "200%")];
            let q = quality.iter().map(|&(v, _)| v).min_by(|a, b| (a - self.prefs.render_scale).abs().total_cmp(&(b - self.prefs.render_scale).abs())).unwrap_or(1.0);
            view_body = view_body.push(labeled(
                "Render quality",
                format!("{}×{} px", self.render_px(1.0).0, self.render_px(1.0).1),
                segmented_f(&quality, q, ViewerMsg::Quality3),
            ));
            if self.v3.blocks {
                let d = self.prefs.block_distance;
                view_body = view_body.push(labeled(
                    "Block distance",
                    format!("{d} blocks ({} chunks)", d / 16),
                    slider(64..=if self.v3.gpu { 384 } else { crate::viewer_blocks::RADIUS }, d, ViewerMsg::BlockDist3).step(32).into(),
                ));
            }
            {
                use crate::viewer_3d_input::{Controls3, Move3};
                let p = &self.prefs;
                view_body = view_body
                    .push(labeled("Movement", String::new(), segmented(&[(Move3::Normal, "Normal"), (Move3::Spectator, "Spectator")], p.movement, ViewerMsg::Move3)))
                    .push(text(if p.movement == Move3::Normal { "Stays above the ground and block tops." } else { "Flies through blocks, like spectator mode." }).size(11).color(style::muted()))
                    .push(labeled("Controls", String::new(), segmented(&[(Controls3::Drag, "Drag"), (Controls3::MouseLook, "Mouse look")], p.controls, ViewerMsg::Controls3)))
                    .push(
                        text(if p.controls == Controls3::Drag {
                            "Left-drag looks, right-drag moves."
                        } else {
                            "Click the view to capture the mouse, then look around like in game. Esc releases it."
                        })
                        .size(11)
                        .color(style::muted()),
                    )
                    .push(labeled("Look sensitivity", format!("{:.1}×", p.look_sens), slider(0.2..=3.0, p.look_sens, ViewerMsg::LookSens3).step(0.1f32).into()))
                    .push(labeled("Fly speed", format!("{:.0} blocks/s", p.fly_speed), slider(2.0..=200.0, p.fly_speed, ViewerMsg::FlySpeed3).step(1.0f32).into()))
                    .push(text("W A S D move · Space up · Shift down · Ctrl faster").size(11).color(style::hint()));
            }
            view_body = view_body
                .push(labeled("View distance", format!("{:.0} blocks", c.far), slider(200.0..=if self.v3.mesh.is_some() { 4000.0 } else { 1600.0 }, c.far, ViewerMsg::Far3).step(50.0f32).into()))
                .push(labeled("Height scale", format!("{:.1}×", c.scale), slider(0.5..=3.0, c.scale, ViewerMsg::Scale3).step(0.1f32).into()))
                .push(text(format!("Camera {:.0}, {:.0} · Y {:.0} · heading {:.0}°", c.x, c.z, c.y, c.yaw.to_degrees().rem_euclid(360.0))).size(11).color(style::hint()));
        }
        let col = column![
            section(Icon::Layers, "View", view_body.into()),
            section(Icon::Worlds, "Dimension", dims),
            section(Icon::Map, "Map style", styles.into()),
            section(Icon::Lighting, "Lighting", light.into()),
            section(Icon::Layers, "Layers", layers.into()),
            section(Icon::Settings, "Rendering", render.into()),
            section(Icon::Spawn, "Navigate", nav.into()),
            section(Icon::Bookmark, "Bookmarks", marks.into()),
            export,
        ]
        .spacing(10)
        .padding(10);
        container(scrollable(col).height(Length::Fill)).width(320).height(Length::Fill).style(style::rail()).into()
    }

    fn overlay(&self, bookmarks: &[Bookmark], spawn: Option<(i32, i32)>) -> Overlay {
        let mut markers = Vec::new();
        let p = &self.prefs;
        if p.spawn
            && self.ctl.dim == 0
            && let Some((x, z)) = spawn
        {
            markers.push(Marker { x: x as f64 + 0.5, z: z as f64 + 0.5, label: "Spawn".into(), icon: Some(icon_handle(Icon::Spawn)), color: Color::WHITE });
        }
        if p.players {
            for pl in self.players.iter().filter(|pl| pl.dim == self.ctl.dim) {
                markers.push(Marker { x: pl.x, z: pl.z, label: pl.label.clone(), icon: Some(icon_handle(Icon::Player)), color: style::t().warn });
            }
        }
        if p.bookmarks {
            for (_, b) in self.my_bookmarks(bookmarks).into_iter().filter(|(_, b)| b.dim == self.ctl.dim) {
                markers.push(Marker { x: b.x as f64 + 0.5, z: b.z as f64 + 0.5, label: b.name.clone(), icon: Some(icon_handle(Icon::Bookmark)), color: style::t().info });
            }
        }
        if p.entities {
            use omt_render::markers::{EntityKind, entity_kind};
            let named = self.ctl.camera.zoom >= 3.0;
            for e in self.entities.iter() {
                let color = match entity_kind(&e.id) {
                    EntityKind::Hostile => Color::from_rgb8(0xe5, 0x48, 0x4d),
                    EntityKind::Passive => Color::from_rgb8(0x7a, 0xd0, 0x4f),
                    EntityKind::Villager => Color::from_rgb8(0xf2, 0xc1, 0x4e),
                    EntityKind::Item => Color::from_rgb8(0xa3, 0xa8, 0xb2),
                    EntityKind::Other => Color::from_rgb8(0x5f, 0xe0, 0xe6),
                };
                let label = if named { e.id.trim_start_matches("minecraft:").replace('_', " ") } else { String::new() };
                markers.push(Marker { x: e.x, z: e.z, label, icon: None, color });
            }
        }
        let caption = match self.hover {
            Some((x, z)) => {
                let (bx, bz) = (x.floor() as i32, z.floor() as i32);
                let id = self.ctl.map.as_ref().and_then(|m| m.identify(bx, bz)).map(|(n, y, approx)| format!("   {}{} · Y {y}", if approx { "≈ " } else { "" }, n.trim_start_matches("minecraft:"))).unwrap_or_default();
                let biome = self.ctl.map.as_ref().and_then(|m| m.biome_at(bx, bz)).map(|b| format!(" · {}", b.trim_start_matches("minecraft:").replace('_', " "))).unwrap_or_default();
                let id = format!("{id}{biome}");
                format!("X {bx}  Z {bz}   chunk {}, {}   region {}, {}{id}", bx.div_euclid(16), bz.div_euclid(16), bx.div_euclid(512), bz.div_euclid(512))
            }
            None => String::new(),
        };
        let textured = if self.textured_active() { self.tex.cells.iter().map(|(&(x, z), h)| (x, z, h.clone())).collect() } else { Vec::new() };
        Overlay { caption, markers, chunk_grid: p.chunk_grid, region_grid: p.region_grid, slime: self.slime.clone(), hud: p.hud, textured, textured_cell: crate::viewer_tex::CELL, ruler: p.ruler, measure: self.measure, cursor: self.hover, ..Default::default() }
    }

    pub fn view<'a>(&'a self, bookmarks: &'a [Bookmark], spawn: Option<(i32, i32)>) -> Element<'a, ViewerMsg> {
        let map: Element<'a, ViewerMsg> = match &self.ctl.map {
            Some(_) if self.v3.on => {
                let status = if self.v3.frame.is_none() {
                    "building 3D terrain…".to_string()
                } else if self.v3.bbuilding {
                    "loading blocks around the camera…".to_string()
                } else if self.v3.building {
                    "loading more terrain…".to_string()
                } else {
                    String::new()
                };
                iced::widget::container(iced::widget::canvas(crate::viewer_3d::Canvas3 {
                    frame: self.v3.frame.as_ref(),
                    crisp: self.v3.gpu && (self.prefs.render_scale - 1.0).abs() < 0.01,
                    mouse_look: self.prefs.controls == crate::viewer_3d_input::Controls3::MouseLook,
                    locked: self.v3.locked,
                    status, on_event: Box::new(ViewerMsg::Ev3) }).width(Length::Fill).height(Length::Fill)).clip(true).into()
            }
            Some(map) => crate::mapview::element(MapView { map, camera: self.ctl.camera, overlay: self.overlay(bookmarks, spawn), on_event: Box::new(ViewerMsg::Map) }),
            None => container(column![assets::block(omt_art::Material::GrassTop, 72.0), text(if self.ctl.loading { self.ctl.status.as_str() } else { "No map" }).color(style::muted())].spacing(12).align_x(Alignment::Center))
                .center(Length::Fill)
                .style(move |_| container::Style { background: Some(iced::Background::Color(style::t().map_bg)), ..Default::default() })
                .into(),
        };
        let dim = ["Overworld", "Nether", "The End"].get(self.ctl.dim as usize).copied().unwrap_or("?");
        let zoom = if self.v3.on { "3D".to_string() } else { format!("zoom {}", zoom_label(self.ctl.camera.zoom)) };
        let chip = container(
            row![
                assets::icon(Icon::Map, 16.0),
                text(format!("{dim} · {}", self.prefs.style.style.name())).size(12),
                text(format!("· {zoom}")).size(12).color(Color::from_rgb(0.8, 0.82, 0.86)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([6, 10])
        .style(style::scrim(0.55));
        let hud = container(chip).padding(12);
        row![self.sidebar(bookmarks), stack![map, hud]].height(Length::Fill).into()
    }
}

fn zoom_label(z: f32) -> String {
    if z >= 1.0 { format!("{z:.0}:1") } else { format!("1:{:.0}", 1.0 / z) }
}

fn icon_handle(i: Icon) -> iced::widget::image::Handle {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static C: OnceLock<Mutex<HashMap<Icon, iced::widget::image::Handle>>> = OnceLock::new();
    let m = C.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    g.entry(i)
        .or_insert_with(|| {
            let img = omt_art::icon(i);
            iced::widget::image::Handle::from_rgba(img.w, img.h, img.rgba)
        })
        .clone()
}


/// Segmented picker over float choices (exact match on the given values).
fn segmented_f<'a>(items: &[(f32, &'static str)], current: f32, on: fn(f32) -> ViewerMsg) -> Element<'a, ViewerMsg> {
    items
        .iter()
        .fold(row![].spacing(4), |r, &(v, label)| {
            let active = (v - current).abs() < 0.001;
            r.push(button(text(label).size(11)).padding([4, 6]).on_press(on(v)).style(style::button_style(if active { Btn::Primary } else { Btn::Ghost })))
        })
        .into()
}
