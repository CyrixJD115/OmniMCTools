//! Map Viewer UI, uNmINeD-style: a compact **nav bar** above the map
//! (2D/3D, dimension, map style, go-to, tools), a slim **tool rail**
//! (quick view modes + camera, docked left of the canvas), and a
//! **tabbed side panel** (world data · look · actions) that can collapse
//! to its icon strip. Every control has one home: quick view modes and the
//! camera on the rail, go-to in the nav, everything else in the panel.

use iced::widget::{button, column, container, pick_list, row, scrollable, slider, space, stack, text, text_input, toggler};
use iced::{Alignment, Color, Element, Length};
use omt_art::{Glyph, Icon};
use omt_render::style::MapStyle;
use omt_world::Dimension;

use crate::assets;
use crate::mapconfig::MapQuality;
use crate::mapview::{MapView, Marker, Overlay};
use crate::style::{self, Btn};
use crate::viewer::{Bookmark, Layer, PanelTab, ViewerMsg, ViewerState};

fn labeled<'a>(label: &'a str, value: String, control: Element<'a, ViewerMsg>) -> Element<'a, ViewerMsg> {
    column![row![text(label).size(12).color(style::muted()), space::horizontal(), text(value).size(12)], control].spacing(4).into()
}

fn switch<'a>(label: &'a str, on: bool, f: impl Fn(bool) -> ViewerMsg + 'a) -> Element<'a, ViewerMsg> {
    row![text(label).size(12).width(Length::Fill), toggler(on).size(16).on_toggle(f)].align_y(Alignment::Center).into()
}

fn heading(title: &str) -> Element<'_, ViewerMsg> {
    text(title).size(10).font(style::BOLD).color(style::hint()).into()
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

/// A dimension value for the nav pick list (shows its name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DimChoice(u8);

impl std::fmt::Display for DimChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(Dimension::from_id(self.0).map_or("?", Dimension::name))
    }
}

impl ViewerState {
    // ---------------------------------------------------------------- nav
    fn nav(&self, spawn: Option<(i32, i32)>) -> Element<'_, ViewerMsg> {
        let cur = match (self.v3.on, self.v3.iso) {
            (false, _) => 0u8,
            (true, None) => 1,
            (true, Some(_)) => 2,
        };
        let rot = self.v3.iso_cam.rot;
        let mode = segmented(&[(0u8, "2D"), (1, "3D"), (2, "Iso")], cur, move |m| match m {
            0 => ViewerMsg::Mode3D(false),
            1 => ViewerMsg::Mode3D(true),
            _ => ViewerMsg::Iso(Some(rot)),
        });
        let dims = pick_list(
            Dimension::ALL.map(|d| DimChoice(d.id())),
            Some(DimChoice(self.ctl.dim)),
            |d: DimChoice| ViewerMsg::Dim(d.0),
        )
        .text_size(12)
        .width(112);
        let styles = pick_list(MapStyle::ALL.to_vec(), Some(self.prefs.style.style), ViewerMsg::Style).text_size(12).width(136);
        let spawn_btn = |s: Option<(i32, i32)>| {
            button(assets::glyph(Glyph::Spawn, style::fg(), 16.0))
                .padding(5)
                .on_press_maybe(s.filter(|_| self.ctl.dim == 0).map(|_| ViewerMsg::GotoSpawn))
                .style(style::button_style(Btn::Standard))
        };
        let player_btn = button(assets::glyph(Glyph::Player, style::fg(), 16.0)).padding(5).on_press(ViewerMsg::GotoPlayer).style(style::button_style(Btn::Standard));
        let goto_row = row![
            text_input("X", &self.goto.0).on_input(ViewerMsg::GotoX).on_submit(ViewerMsg::Goto).size(12).width(74).style(style::input()),
            text_input("Z", &self.goto.1).on_input(ViewerMsg::GotoZ).on_submit(ViewerMsg::Goto).size(12).width(74).style(style::input()),
            button(assets::glyph(Glyph::Target, style::t().on_accent, 14.0)).padding(6).on_press(ViewerMsg::Goto).style(style::button_style(Btn::Primary)),
            spawn_btn(spawn),
            player_btn,
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        let tools = self.tools_menu_button();
        container(
            row![mode, dims, styles, space::horizontal(), goto_row, self.cpu_picker(), tools]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .padding([6, 8])
        .width(Length::Fill)
        .style(style::topbar())
        .into()
    }

    /// The compact "Tools ▾" menu (other tools for this world, was the top
    /// bar's row of buttons).
    fn tools_menu_button(&self) -> Element<'_, ViewerMsg> {
        let open = self.tools_menu;
        let label = row![text("Tools").size(12), assets::glyph(Glyph::ChevronDown, style::muted(), 10.0)].spacing(4).align_y(Alignment::Center);
        button(label).padding([6, 10]).on_press(ViewerMsg::ToolsMenu(!open)).style(style::button_style(if open { Btn::ChipOn } else { Btn::Standard })).into()
    }

    fn tools_menu(&self) -> Element<'_, ViewerMsg> {
        let items = [
            crate::session::App::WorldEditor,
            crate::session::App::NbtEditor,
            crate::session::App::Pruner,
            crate::session::App::Converter,
        ];
        let list = items.into_iter().fold(column![].spacing(2), |c, app| {
            let mut r = row![assets::icon(app.icon(), 15.0), text(app.name()).size(12)].spacing(8).align_y(Alignment::Center);
            if app.alpha() {
                r = r.push(style::alpha_badge());
            }
            c.push(button(r).width(196).padding([7, 10]).on_press(ViewerMsg::OpenIn(app)).style(style::button_style(Btn::Nav)))
        });
        container(list)
            .padding(6)
            .style(style::card())
            .into()
    }

    // ---------------------------------------------------------- tool rail
    /// Photoshop-style rail — same anatomy as the Pruner's (`maprail`):
    /// the quick **view modes** (what the canvas shows), with the camera
    /// (zoom/fit) pinned at the bottom. It is deliberately short: markers,
    /// grids and overlays live once, in the panel's **Layers** tab — the
    /// rail is the shortcut, not a second home for them.
    fn tool_rail(&self) -> Element<'_, ViewerMsg> {
        use crate::maprail::{self, Side};
        let p = &self.prefs;
        let side = Side::Left;
        let tex_tip = if crate::textures::current().is_some() { "Block textures when zoomed in" } else { "Block textures (no pack — see Settings)" };
        let top: Vec<Element<'_, ViewerMsg>> = vec![
            maprail::tool(Glyph::Cave, "Underground (caves)", self.cave, Some(ViewerMsg::Cave(!self.cave)), side),
            maprail::tool(Glyph::Slicer, "Slicer (hide blocks above Y)", self.slicer.is_some(), Some(ViewerMsg::Slicer(self.slicer.is_none())), side),
            maprail::tool(Glyph::Texture, tex_tip, p.textured, Some(ViewerMsg::Layer(Layer::Textures, !p.textured)), side),
        ];
        let bottom = maprail::nav_group(ViewerMsg::Zoom(2.0), ViewerMsg::Zoom(0.5), ViewerMsg::Fit, true);
        maprail::strip(top, bottom)
    }

    // -------------------------------------------------------- side panel
    /// The panel's icon strip (right-docked, tooltips open left), grouped
    /// world data · look · actions; the collapse chevron is pinned to the
    /// bottom like the rail's nav group.
    fn panel_tab_strip(&self) -> Element<'_, ViewerMsg> {
        use crate::maprail::{self, Side};
        // Separator before these tabs (after the data and look groups).
        const GROUP_STARTS: [PanelTab; 2] = [PanelTab::View, PanelTab::Layers];
        let mut top: Vec<Element<'_, ViewerMsg>> = Vec::new();
        for tab in PanelTab::ALL {
            if GROUP_STARTS.contains(&tab) {
                top.push(maprail::separator());
            }
            top.push(maprail::tool(tab.glyph(), tab.name(), self.prefs.panel() == Some(tab), Some(ViewerMsg::Panel(Some(tab))), Side::Right));
        }
        let (glyph, tip_label, msg) = if self.prefs.panel().is_some() {
            (Glyph::ChevronRight, "Collapse panel", ViewerMsg::Panel(None))
        } else {
            (Glyph::ChevronLeft, "Expand panel", ViewerMsg::Panel(Some(PanelTab::View)))
        };
        maprail::strip(top, vec![maprail::tool(glyph, tip_label, false, Some(msg), Side::Right)])
    }

    fn panel<'a>(&'a self, bookmarks: &'a [Bookmark]) -> Element<'a, ViewerMsg> {
        let Some(tab) = self.prefs.panel() else { return space::vertical().width(0).height(Length::Fill).into() };
        let body: Element<'a, ViewerMsg> = if let Some(b) = self.ext_tab() { b } else { match tab {
            PanelTab::View => self.view_tab(),
            PanelTab::Style => self.style_tab(),
            PanelTab::Layers => self.layers_tab(),
            PanelTab::Find => self.find_tab(bookmarks),
            PanelTab::Export => self.export_tab(),
            _ => self.view_tab(),
        } };
        let head = row![
            text(tab.name()).size(12).font(style::BOLD),
            space::horizontal(),
            button(assets::glyph(Glyph::Close, style::hint(), 10.0)).padding(4).on_press(ViewerMsg::Panel(None)).style(style::button_style(Btn::Ghost)),
        ]
        .align_y(Alignment::Center);
        container(column![head, scrollable(column![body].width(Length::Fill).padding(iced::Padding { right: 12.0, ..iced::Padding::ZERO })).height(Length::Fill)].spacing(8))
            .padding([8, 10])
            .width(262)
            .height(Length::Fill)
            .style(style::rail())
            .into()
    }

    fn view_tab(&self) -> Element<'_, ViewerMsg> {
        let mut col = column![].spacing(8);
        if let Some(rot) = self.v3.iso {
            const ANGLES: [&str; 4] = ["North-west", "North-east", "South-east", "South-west"];
            col = col.push(heading("ISOMETRIC VIEW"));
            col = col.push(labeled("View angle", ANGLES[rot as usize % 4].to_string(), segmented(&[(0u8, "NW"), (1, "NE"), (2, "SE"), (3, "SW")], rot, |r| ViewerMsg::Iso(Some(r)))));
            col = col.push(text(format!("Zoom: {:.2} px per block · drag to pan, wheel to zoom", self.v3.iso_cam.scale)).size(11).color(style::muted()));
            return col.into();
        }
        if !self.v3.on {
            let c = crate::mapconfig::config();
            col = col.push(heading("RENDERING"));
            col = col.push(labeled("Quality", String::new(), pick_list(MapQuality::ALL.to_vec(), Some(c.quality), move |q| ViewerMsg::MapUi(crate::mapdata::MapUiMsg::Quality(q))).text_size(12).width(Length::Fill).into()));
            col = col.push(labeled("Detail bias", String::new(), pick_list(crate::mapconfig::DetailBias::ALL.to_vec(), Some(crate::mapconfig::DetailBias(c.detail_bias)), move |d: crate::mapconfig::DetailBias| ViewerMsg::MapUi(crate::mapdata::MapUiMsg::Detail(d.0))).text_size(12).width(Length::Fill).into()));
            col = col.push(switch("Smooth scaling", c.smooth, move |v| ViewerMsg::MapUi(crate::mapdata::MapUiMsg::Smooth(v))));
            col = col.push(text("Detail bias trades tile sharpness for speed; quality caps the chunk reads per tile.").size(11).color(style::muted()));
            return col.into();
        }
        let c = &self.v3.cam;
        let has = if self.v3.blocks { self.v3.bmesh.is_some() } else { self.v3.mesh.is_some() };
        let gpu_label = match (has.then_some(()), self.v3.gpu) {
            (Some(_), true) => format!("GPU · {}", omt_gpu::gpu().map(|g| g.adapter_name.clone()).unwrap_or_default()),
            (None, true) => "GPU (not available — CPU)".to_string(),
            _ => "GPU (off — CPU ray-casting)".to_string(),
        };
        col = col.push(row![text(gpu_label).size(12).width(Length::Fill), toggler(self.v3.gpu).size(16).on_toggle(ViewerMsg::Gpu3)].align_y(Alignment::Center));
        col = col.push(labeled("Detail", String::new(), segmented(&[(false, "Terrain (fast)"), (true, "Blocks (textured)")], self.v3.blocks, ViewerMsg::Detail3)));
        let quality = [(0.5f32, "50%"), (0.75, "75%"), (1.0, "100%"), (1.5, "150%"), (2.0, "200%")];
        let q = quality.iter().map(|&(v, _)| v).min_by(|a, b| (a - self.prefs.render_scale).abs().total_cmp(&(b - self.prefs.render_scale).abs())).unwrap_or(1.0);
        col = col.push(labeled("Render quality", format!("{}×{} px", self.render_px(1.0).0, self.render_px(1.0).1), segmented_f(&quality, q, ViewerMsg::Quality3)));
        if self.v3.blocks {
            let d = self.prefs.block_distance;
            col = col.push(labeled("Block distance", format!("{d} blocks ({} chunks)", d / 16), slider(64..=if self.v3.gpu { 384 } else { crate::viewer_blocks::RADIUS }, d, ViewerMsg::BlockDist3).step(32).into()));
        }
        col = col.push(heading("MOVEMENT"));
        col = col
            .push(labeled("Mode", String::new(), segmented(&[(crate::viewer_3d_input::Move3::Normal, "Normal"), (crate::viewer_3d_input::Move3::Spectator, "Spectator")], self.prefs.movement, ViewerMsg::Move3)))
            .push(text(if self.prefs.movement == crate::viewer_3d_input::Move3::Normal { "Stays above the ground and block tops." } else { "Flies through blocks, like spectator mode." }).size(11).color(style::muted()))
            .push(labeled("Controls", String::new(), segmented(&[(crate::viewer_3d_input::Controls3::Drag, "Drag"), (crate::viewer_3d_input::Controls3::MouseLook, "Mouse look")], self.prefs.controls, ViewerMsg::Controls3)))
            .push(
                text(if self.prefs.controls == crate::viewer_3d_input::Controls3::Drag {
                    "Left-drag looks, right-drag moves."
                } else {
                    "Click the view to capture the mouse, then look around like in game. Esc releases it."
                })
                .size(11)
                .color(style::muted()),
            )
            .push(labeled("Look sensitivity", format!("{:.1}×", self.prefs.look_sens), slider(0.2..=3.0, self.prefs.look_sens, ViewerMsg::LookSens3).step(0.1f32).into()))
            .push(labeled("Fly speed", format!("{:.0} blocks/s", self.prefs.fly_speed), slider(2.0..=200.0, self.prefs.fly_speed, ViewerMsg::FlySpeed3).step(1.0f32).into()))
            .push(text("W A S D move · Space up · Shift down · Ctrl faster").size(11).color(style::hint()));
        col = col.push(heading("CAMERA"));
        col = col
            .push(labeled("View distance", format!("{:.0} blocks", c.far), slider(200.0..=if self.v3.mesh.is_some() { 4000.0 } else { 1600.0 }, c.far, ViewerMsg::Far3).step(50.0f32).into()))
            .push(labeled("Height scale", format!("{:.1}×", c.scale), slider(0.5..=3.0, c.scale, ViewerMsg::Scale3).step(0.1f32).into()))
            .push(text(format!("Camera {:.0}, {:.0} · Y {:.0} · heading {:.0}°", c.x, c.z, c.y, c.yaw.to_degrees().rem_euclid(360.0))).size(11).color(style::hint()));
        col.into()
    }

    fn style_tab(&self) -> Element<'_, ViewerMsg> {
        let s = self.prefs.style;
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
            labeled("Cast shadows", if s.shadows <= 0.0 { "off".into() } else { format!("{:.0}%", s.shadows * 100.0) }, slider(0.0..=1.0, s.shadows, ViewerMsg::Shadows).step(0.05f32).into()),
        ]
        .spacing(8);
        if s.style == MapStyle::Topographic {
            light = light.push(labeled("Contour every", format!("{} blocks", s.contour), slider(0..=64, s.contour, ViewerMsg::Contour).step(4).into()));
        }
        if s.style == MapStyle::Biomes && !self.legend.is_empty() {
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
        column![heading("MAP STYLE"), styles, heading("LIGHTING"), light].spacing(8).into()
    }

    fn layers_tab(&self) -> Element<'_, ViewerMsg> {
        let p = &self.prefs;
        let ent_label = if self.entities.is_empty() { "Entities (zoom in)".to_string() } else { format!("Entities ({} in view)", self.entities.len()) };
        let slime_note = if !self.bedrock && self.seed.is_none() { " (seed unknown)" } else { "" };
        let mut col = column![heading("VIEW MODES"), switch("Underground (caves)", self.cave, ViewerMsg::Cave)].spacing(8);
        col = col.push(switch("Slicer", self.slicer.is_some(), ViewerMsg::Slicer));
        let (lo, hi) = if self.ctl.dim == 1 { (0, 127) } else { (-64, 319) };
        // uNmINeD's slicer steps: one block / one section.
        let steps = |y: i32, f: fn(i32) -> ViewerMsg| -> Element<'_, ViewerMsg> {
            [-16, -1, 1, 16]
                .iter()
                .fold(row![].spacing(4), |r, &d| r.push(button(text(format!("{d:+}")).size(11).center()).width(Length::Fill).padding([3, 4]).on_press(f((y + d).clamp(lo, hi))).style(style::button_style(Btn::Chip))))
                .into()
        };
        if let Some(y) = self.slicer {
            col = col.push(labeled("Hide blocks above Y", y.to_string(), slider(lo..=hi, y, ViewerMsg::SlicerY).into())).push(steps(y, ViewerMsg::SlicerY));
        }
        col = col.push(switch("Hide below", self.slicer_min.is_some(), move |v| ViewerMsg::SlicerMin(v.then_some(0.max(lo)))));
        if let Some(m) = self.slicer_min {
            col = col.push(labeled("Hide blocks below Y", m.to_string(), slider(lo..=hi, m, |v| ViewerMsg::SlicerMin(Some(v))).into())).push(steps(m, |v| ViewerMsg::SlicerMin(Some(v))));
        }
        col = col.push(switch("Biome colors", p.biome_tint, |v| ViewerMsg::Layer(Layer::BiomeTint, v)));
        col = col.push(switch(if crate::textures::current().is_some() { "Block textures when zoomed in" } else { "Block textures (no pack — see Settings)" }, p.textured, |v| ViewerMsg::Layer(Layer::Textures, v)));
        col = col.push(heading("GRIDS"));
        col = col
            .push(labeled("Chunk grid", String::new(), segmented(&[(None, "Auto"), (Some(true), "On"), (Some(false), "Off")], p.chunk_grid, ViewerMsg::ChunkGrid)))
            .push(labeled("Rulers", String::new(), segmented(&crate::mapview_overlay::Ruler::ALL.map(|r| (r, r.name())), p.ruler, ViewerMsg::Ruler)))
            .push(switch("Region grid (32×32 chunks)", p.region_grid, |v| ViewerMsg::Layer(Layer::RegionGrid, v)))
            .push(switch("Region coordinates on the grid", p.region_labels, |v| ViewerMsg::Layer(Layer::RegionLabels, v)))
            .push(switch("World border (Java)", p.border, |v| ViewerMsg::Layer(Layer::Border, v)));
        col = col.push(heading("MARKERS"));
        col = col
            .push(switch(if slime_note.is_empty() { "Slime chunks" } else { "Slime chunks (seed unknown)" }, p.slime, |v| ViewerMsg::Layer(Layer::Slime, v)))
            .push(switch("World spawn", p.spawn, |v| ViewerMsg::Layer(Layer::Spawn, v)))
            .push(switch("Players", p.players, |v| ViewerMsg::Layer(Layer::Players, v)))
            .push(switch("Bookmarks", p.bookmarks, |v| ViewerMsg::Layer(Layer::Bookmarks, v)))
            .push(row![text(ent_label).size(12).width(Length::Fill), toggler(p.entities).size(16).on_toggle(|v| ViewerMsg::Layer(Layer::Entities, v))].align_y(Alignment::Center))
            .push(switch("Scale bar & compass", p.hud, |v| ViewerMsg::Layer(Layer::Hud, v)));
        col.into()
    }

    fn find_tab<'a>(&'a self, bookmarks: &'a [Bookmark]) -> Element<'a, ViewerMsg> {
        // Go-to lives once, in the top bar (X / Z boxes); this tab holds the
        // copy commands for the picked spot and the bookmarks.
        let nav = column![
            text("Pick a spot with the X / Z boxes in the top bar — click the map fills them (or type “x z”, “spawn”, “player”). Left-drag measures a distance. Arrows pan, +/− zoom, * resets, Ctrl+G cycles the grid.").size(11).color(style::hint()),
            row![button(text("Copy /tp").size(11)).padding([5, 8]).on_press(ViewerMsg::CopyTp).style(style::button_style(Btn::Standard)), space::horizontal()].align_y(Alignment::Center),
            self.copy_actions(),
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
            heading("BOOKMARKS"),
            row![
                text_input("Name this place…", &self.bookmark_name).on_input(ViewerMsg::BookmarkName).on_submit(ViewerMsg::AddBookmark).size(12).style(style::input()),
                button(text("Add").size(12)).on_press(ViewerMsg::AddBookmark).style(style::button_style(Btn::Primary)),
            ]
            .spacing(6),
            list,
        ]
        .spacing(8);
        column![heading("COORDINATES"), nav, marks].spacing(8).into()
    }

    fn export_tab(&self) -> Element<'_, ViewerMsg> {
        let dim = Dimension::from_id(self.ctl.dim).map_or("?", Dimension::name);
        let zoom = if self.v3.on { "3D".to_string() } else { zoom_label(self.ctl.camera.zoom) };
        let hover = self.hover.map_or("—".to_string(), |(x, z)| format!("{:.0}, {:.0}", x.floor(), z.floor()));
        let export = button(row![assets::glyph(Glyph::Export, style::fg(), 14.0), text("Export view as PNG…").size(12)].spacing(8).align_y(Alignment::Center))
            .width(Length::Fill)
            .on_press(ViewerMsg::Export)
            .style(style::button_style(Btn::Standard));
        column![
            heading("EXPORT VIEW"),
            export,
            text("Exports exactly what the canvas shows, at its pixel size.").size(11).color(style::muted()),
            heading("CURRENT VIEW"),
            column![
                text(format!("Dimension: {dim}")).size(12),
                text(format!("Map style: {}", self.prefs.style.style.name())).size(12),
                text(format!("Zoom: {zoom}")).size(12),
                text(format!("Cursor: {hover}")).size(12),
                text(format!("Chunks loaded: {}", self.ctl.map.as_ref().map_or(0, |m| m.tiles.len()))).size(12),
            ]
            .spacing(4),
            self.export_area_ui(),
            self.export_web_ui(),
        ]
        .spacing(8)
        .into()
    }

    // ------------------------------------------------------------ overlay
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
            // Named server players once loaded; the quick markers before.
            if self.ext.players.is_empty() {
                for pl in self.players.iter().filter(|pl| pl.dim == self.ctl.dim) {
                    markers.push(Marker { x: pl.x, z: pl.z, label: pl.label.clone(), icon: Some(icon_handle(Icon::Player)), color: style::t().warn });
                }
            } else {
                // Crowded servers: names only once zoomed in (icons always).
                let crowd = self.ext.players.iter().filter(|pl| pl.dim == self.ctl.dim).count() > 30 && self.ctl.camera.zoom < 2.0;
                for pl in self.ext.players.iter().filter(|pl| pl.dim == self.ctl.dim) {
                    let label = if crowd { String::new() } else { pl.label() };
                    markers.push(Marker { x: pl.x, z: pl.z, label, icon: Some(icon_handle(Icon::Player)), color: style::t().warn });
                }
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
        let mut ov = Overlay { caption, markers, chunk_grid: p.chunk_grid, region_grid: p.region_grid, slime: self.slime.clone(), hud: p.hud, textured, textured_cell: crate::viewer_tex::CELL, ruler: p.ruler, measure: self.measure, cursor: self.hover, ..Default::default() };
        self.ext_overlay(&mut ov);
        ov
    }

    // --------------------------------------------------------------- view
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
                    mouse_look: self.prefs.controls == crate::viewer_3d_input::Controls3::MouseLook && self.v3.iso.is_none(),
                    locked: self.v3.locked,
                    iso: self.v3.iso.is_some(),
                    status, on_event: Box::new(ViewerMsg::Ev3) }).width(Length::Fill).height(Length::Fill)).clip(true).into()
            }
            Some(map) => crate::mapview::element(MapView { map, camera: self.ctl.camera, overlay: self.overlay(bookmarks, spawn), on_event: Box::new(ViewerMsg::Map) }),
            None => container(column![assets::block(omt_art::Material::GrassTop, 72.0), text(if self.ctl.loading { self.ctl.status.as_str() } else { "No map" }).color(style::muted())].spacing(12).align_x(Alignment::Center))
                .center(Length::Fill)
                .style(move |_| container::Style { background: Some(iced::Background::Color(style::t().map_bg)), ..Default::default() })
                .into(),
        };
        let dim = Dimension::from_id(self.ctl.dim).map_or("?", Dimension::name);
        let zoom = match (self.v3.on, self.v3.iso) {
            (true, Some(_)) => format!("isometric · {:.2} px/block", self.v3.iso_cam.scale),
            (true, None) => "3D".to_string(),
            _ => format!("zoom {}", zoom_label(self.ctl.camera.zoom)),
        };
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
        // Floating "Tools" menu, anchored under the nav's right side.
        let menu: Element<'a, ViewerMsg> = if self.tools_menu {
            container(self.tools_menu()).align_y(Alignment::Start).align_x(Alignment::End).padding([44, 8]).width(Length::Fill).height(Length::Fill).into()
        } else {
            space::vertical().into()
        };
        row![
            self.tool_rail(),
            column![self.nav(spawn), stack![map, hud, menu]].width(Length::Fill).height(Length::Fill),
            self.panel_tab_strip(),
            self.panel(bookmarks),
        ]
        .height(Length::Fill)
        .into()
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
