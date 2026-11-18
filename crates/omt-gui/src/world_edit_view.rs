//! World Editor UI: tool panel (selection, Y range, tool forms with block
//! suggestions showing real textures, history) beside the map with the
//! selection box drawn on it.

use iced::widget::{button, column, container, row, scrollable, slider, space, text, text_input, toggler};
use iced::{Alignment, Element, Length};
use omt_art::{Glyph, Icon};

use crate::assets;
use crate::mapview::{MapView, Outline, Overlay};
use crate::style::{self, Btn};
use crate::world_edit::{EditMsg, EditorState, Field, Tool};

fn card<'a>(icon: Icon, title: &'a str, body: Element<'a, EditMsg>) -> Element<'a, EditMsg> {
    container(column![row![assets::icon(icon, 16.0), text(title).size(13).font(style::BOLD)].spacing(8).align_y(Alignment::Center), body].spacing(10))
        .padding(12)
        .style(style::card_flat())
        .into()
}

fn block_field<'a>(label: &'a str, value: &'a str, f: Field) -> Element<'a, EditMsg> {
    row![
        assets::block_icon(value.trim(), 22.0),
        column![
            text(label).size(11).color(style::muted()),
            text_input("minecraft:stone", value).on_input(move |s| EditMsg::Block(f, s)).size(12).style(style::input()),
        ]
        .spacing(2)
        .width(Length::Fill),
    ]
    .spacing(8)
    .align_y(Alignment::End)
    .into()
}

impl EditorState {
    fn suggestions(&self) -> Element<'_, EditMsg> {
        let q = match self.focus {
            Field::Fill => &self.fill,
            Field::From => &self.from,
            Field::To => &self.to,
        };
        let q = q.trim().trim_start_matches("minecraft:").to_lowercase();
        if q.len() < 2 {
            return space::vertical().height(0).into();
        }
        let names: Vec<&str> = omt_render::Palette::embedded().block_names().into_iter().filter(|n| n.contains(q.as_str()) && !n.trim_start_matches("minecraft:").eq(q.as_str())).take(12).collect();
        names
            .into_iter()
            .fold(row![].spacing(4), |r, n| {
                r.push(
                    button(row![assets::block_icon(n, 16.0), text(n.trim_start_matches("minecraft:").to_string()).size(11)].spacing(4).align_y(Alignment::Center))
                        .padding([3, 6])
                        .on_press(EditMsg::Pick(n.to_string()))
                        .style(style::button_style(Btn::Chip)),
                )
            })
            .wrap()
            .vertical_spacing(4)
            .into()
    }

    fn tool_form(&self) -> Element<'_, EditMsg> {
        let busy = self.busy.is_some();
        let apply = |label: &'static str, danger: bool| -> Element<'_, EditMsg> {
            button(text(label).size(13)).width(Length::Fill).padding([8, 12]).on_press_maybe((!busy && self.sel().is_some()).then_some(EditMsg::Apply)).style(style::button_style(if danger { Btn::Danger } else { Btn::Primary })).into()
        };
        match self.tool {
            Tool::Fill => column![
                block_field("Fill with", &self.fill, Field::Fill),
                self.suggestions(),
                row![text("Only replace air").size(12).width(Length::Fill), toggler(self.only_air).size(16).on_toggle(EditMsg::OnlyAir)].align_y(Alignment::Center),
                apply("Fill selection", false),
            ]
            .spacing(8)
            .into(),
            Tool::Replace => column![
                block_field("Replace (name or name[prop=value])", &self.from, Field::From),
                block_field("With", &self.to, Field::To),
                self.suggestions(),
                apply("Replace in selection", false),
            ]
            .spacing(8)
            .into(),
            Tool::Clear => column![text("Sets every block in the selection to air.").size(12).color(style::muted()), apply("Clear selection", true)].spacing(8).into(),
            Tool::Count => {
                let total: u64 = self.counts.iter().map(|c| c.1).sum::<u64>().max(1);
                let list = self.counts.iter().take(24).fold(column![].spacing(3), |c, (n, k)| {
                    c.push(
                        row![
                            assets::block_icon(n, 16.0),
                            text(n.trim_start_matches("minecraft:").replace('_', " ")).size(11).width(Length::Fill),
                            text(format!("{k}")).size(11),
                            text(format!("{:.1}%", *k as f64 * 100.0 / total as f64)).size(11).color(style::hint()).width(46),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                });
                column![
                    button(text("Analyze selection").size(13)).width(Length::Fill).padding([8, 12]).on_press_maybe((!busy && self.sel().is_some()).then_some(EditMsg::Count)).style(style::button_style(Btn::Primary)),
                    list,
                ]
                .spacing(8)
                .into()
            }
            Tool::Clipboard => {
                let clip = match &self.clipboard {
                    Some(v) => format!("Clipboard: {}×{}×{} ({} blocks)", v.size[0], v.size[1], v.size[2], v.idx.len()),
                    None => "Clipboard empty".into(),
                };
                column![
                    button(text("Copy selection").size(13)).width(Length::Fill).padding([8, 12]).on_press_maybe((!busy && self.sel().is_some()).then_some(EditMsg::Copy)).style(style::button_style(Btn::Standard)),
                    text(clip).size(11).color(style::muted()),
                    row![
                        text_input("X", &self.paste_at[0]).on_input(|s| EditMsg::PasteAt(0, s)).size(12).style(style::input()),
                        text_input("Y", &self.paste_at[1]).on_input(|s| EditMsg::PasteAt(1, s)).size(12).style(style::input()),
                        text_input("Z", &self.paste_at[2]).on_input(|s| EditMsg::PasteAt(2, s)).size(12).style(style::input()),
                        button(assets::glyph(Glyph::Target, style::fg(), 14.0)).padding(6).on_press(EditMsg::PasteHere).style(style::button_style(Btn::Standard)),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                    row![text("Skip air (merge)").size(12).width(Length::Fill), toggler(self.skip_air).size(16).on_toggle(EditMsg::SkipAir)].align_y(Alignment::Center),
                    row![
                        button(text("⟳ Rotate 90°").size(12)).on_press_maybe(self.clipboard.is_some().then_some(EditMsg::Transform(omt_edit::transform::Xform::RotateCw))).style(style::button_style(Btn::Standard)),
                        button(text("⇋ Flip E–W").size(12)).on_press_maybe(self.clipboard.is_some().then_some(EditMsg::Transform(omt_edit::transform::Xform::FlipX))).style(style::button_style(Btn::Standard)),
                        button(text("⇅ Flip N–S").size(12)).on_press_maybe(self.clipboard.is_some().then_some(EditMsg::Transform(omt_edit::transform::Xform::FlipZ))).style(style::button_style(Btn::Standard)),
                    ]
                    .spacing(4),
                    button(text("Paste").size(13)).width(Length::Fill).padding([8, 12]).on_press_maybe((!busy && self.clipboard.is_some()).then_some(EditMsg::Paste)).style(style::button_style(Btn::Primary)),
                ]
                .spacing(8)
                .into()
            }
            Tool::Entities => {
                let total: u64 = self.entity_counts.iter().map(|c| c.1).sum();
                let list = self.entity_counts.iter().take(20).fold(column![].spacing(3), |c, (id, n)| {
                    let short = id.trim_start_matches("minecraft:").to_string();
                    c.push(
                        row![
                            button(text(short.replace('_', " ")).size(11)).padding([2, 6]).on_press(EditMsg::EntityFilter(id.clone())).style(style::button_style(Btn::Ghost)),
                            space::horizontal(),
                            text(n.to_string()).size(11).color(style::muted()),
                        ]
                        .align_y(Alignment::Center),
                    )
                });
                column![
                    button(text("Scan selection").size(13)).width(Length::Fill).padding([8, 12]).on_press_maybe((!busy && self.sel().is_some()).then_some(EditMsg::ScanEntities)).style(style::button_style(Btn::Standard)),
                    text(if self.entity_counts.is_empty() { "Scan to list the entities inside the selection (click a type to filter).".to_string() } else { format!("{total} entities") }).size(11).color(style::muted()),
                    list,
                    text_input("Type to remove (empty = all, players are kept)", &self.entity_filter).on_input(EditMsg::EntityFilter).size(12).style(style::input()),
                    button(text(if self.entity_filter.trim().is_empty() { "Remove all entities".to_string() } else { format!("Remove {}", self.entity_filter.trim().trim_start_matches("minecraft:")) }).size(13))
                        .width(Length::Fill)
                        .padding([8, 12])
                        .on_press_maybe((!busy && self.sel().is_some()).then_some(EditMsg::RemoveEntities))
                        .style(style::button_style(Btn::Danger)),
                ]
                .spacing(8)
                .into()
            }
            Tool::Biome => {
                let q = self.biome.trim().trim_start_matches("minecraft:").to_lowercase();
                let chips = omt_render::Palette::embedded().biome_names().into_iter().filter(|n| q.len() >= 2 && n.contains(q.as_str()) && n.trim_start_matches("minecraft:") != q).take(12).fold(row![].spacing(4), |r, n| {
                    let c = omt_render::style::biome_color(n);
                    let col = iced::Color::from_rgb8(c[0], c[1], c[2]);
                    r.push(
                        button(row![container(space::horizontal().width(10)).height(10).style(move |_| container::Style { background: Some(iced::Background::Color(col)), border: iced::border::rounded(2), ..Default::default() }), text(n.trim_start_matches("minecraft:").to_string()).size(11)].spacing(4).align_y(Alignment::Center))
                            .padding([3, 6])
                            .on_press(EditMsg::BiomeName(n.to_string()))
                            .style(style::button_style(Btn::Chip)),
                    )
                });
                column![
                    text("Repaints the biome inside the selection (Java: 4×4×4 cells; Bedrock: per block). Grass, leaves and water colors follow in-game.").size(12).color(style::muted()),
                    text_input("minecraft:cherry_grove", &self.biome).on_input(EditMsg::BiomeName).size(12).style(style::input()),
                    chips.wrap().vertical_spacing(4),
                    button(text("Paint biome").size(13)).width(Length::Fill).padding([8, 12]).on_press_maybe((!busy && self.sel().is_some()).then_some(EditMsg::PaintBiome)).style(style::button_style(Btn::Primary)),
                ]
                .spacing(8)
                .into()
            }
            Tool::Structure => column![
                text("Export the clipboard (copy a selection first) or import a structure to paste.").size(12).color(style::muted()),
                button(text("Export .mcstructure (Bedrock)…").size(12)).width(Length::Fill).on_press(EditMsg::Export(true)).style(style::button_style(Btn::Standard)),
                button(text("Export structure .nbt (Java)…").size(12)).width(Length::Fill).on_press(EditMsg::Export(false)).style(style::button_style(Btn::Standard)),
                button(text("Import structure…").size(12)).width(Length::Fill).on_press(EditMsg::Import).style(style::button_style(Btn::Primary)),
            ]
            .spacing(8)
            .into(),
        }
    }

    fn panel(&self) -> Element<'_, EditMsg> {
        let (lo, hi) = omt_edit::y_range(self.ctl.dim);
        let size = self.sel().map(|s| format!("{} × {} × {} = {} blocks", s.size()[0], s.size()[1], s.size()[2], s.volume())).unwrap_or_else(|| "No selection — drag on the map".into());
        let corners = row![
            text_input("X1", &self.corner_text[0]).on_input(|s| EditMsg::Corner(0, s)).size(12).style(style::input()),
            text_input("Z1", &self.corner_text[1]).on_input(|s| EditMsg::Corner(1, s)).size(12).style(style::input()),
            text_input("X2", &self.corner_text[2]).on_input(|s| EditMsg::Corner(2, s)).size(12).style(style::input()),
            text_input("Z2", &self.corner_text[3]).on_input(|s| EditMsg::Corner(3, s)).size(12).style(style::input()),
        ]
        .spacing(6);
        let selection = column![
            corners,
            column![row![text("Bottom Y").size(12).color(style::muted()), space::horizontal(), text(self.y.0.to_string()).size(12)], slider(lo..=hi, self.y.0, EditMsg::YMin)].spacing(2),
            column![row![text("Top Y").size(12).color(style::muted()), space::horizontal(), text(self.y.1.to_string()).size(12)], slider(lo..=hi, self.y.1, EditMsg::YMax)].spacing(2),
            text(size).size(11).color(style::hint()),
        ]
        .spacing(8);
        let tools = Tool::ALL.iter().fold(row![].spacing(4), |r, &t| r.push(button(text(t.name()).size(12)).padding([5, 8]).on_press(EditMsg::Tool(t)).style(style::button_style(if t == self.tool { Btn::ChipOn } else { Btn::Chip })))).wrap().vertical_spacing(4);
        let history = self.history.iter().rev().take(8).fold(column![].spacing(3), |c, (label, _)| c.push(text(format!("• {label}")).size(11).color(style::muted())));
        let undo = button(row![assets::glyph(Glyph::Refresh, style::fg(), 14.0), text(format!("Undo ({})", self.history.len())).size(12)].spacing(6).align_y(Alignment::Center))
            .on_press_maybe((self.busy.is_none() && !self.history.is_empty()).then_some(EditMsg::Undo))
            .style(style::button_style(Btn::Standard));
        let dims = [(0u8, "Overworld"), (1, "Nether"), (2, "End")].iter().fold(row![].spacing(4), |r, &(d, n)| {
            r.push(button(text(n).size(12).center()).width(Length::Fill).padding([5, 4]).on_press(EditMsg::Dim(d)).style(style::button_style(if d == self.ctl.dim { Btn::ChipOn } else { Btn::Chip })))
        });
        let warning = container(
            column![
                row![style::alpha_badge(), text("Experimental editor").size(13).font(style::BOLD)].spacing(8).align_y(Alignment::Center),
                text("The World Editor is an alpha preview: features may change and edge cases (lighting, heightmaps, unusual blocks) are not finished. Edits stay on a workspace copy until Save — keep backups and run `omt verify` before playing.").size(11).color(style::muted()),
            ]
            .spacing(6),
        )
        .padding(12)
        .style(style::badge(style::warn()));
        let col = column![
            warning,
            card(Icon::Worlds, "Dimension", dims.into()),
            card(Icon::Layers, "Selection", selection.into()),
            card(Icon::Extensions, "Tool", column![tools, self.tool_form()].spacing(10).into()),
            card(Icon::Star, "History", column![undo, history, text("Edits apply to the workspace copy; Save writes the world, Close discards.").size(11).color(style::hint())].spacing(8).into()),
        ]
        .spacing(10)
        .padding(10);
        container(scrollable(col).height(Length::Fill)).width(340).height(Length::Fill).style(style::rail()).into()
    }

    fn overlay(&self) -> Overlay {
        let mut outlines = Vec::new();
        if let (Some(a), Some(b)) = (self.a, self.b) {
            let (x0, x1) = (a.0.min(b.0) as f64, a.0.max(b.0) as f64 + 1.0);
            let (z0, z1) = (a.1.min(b.1) as f64, a.1.max(b.1) as f64 + 1.0);
            outlines.push(Outline { points: vec![(x0, z0), (x1, z0), (x1, z1), (x0, z1)], closed: true, active: true, handles: vec![(x0, z0), (x1, z1)] });
        }
        if let (Some(v), Some(x), Some(z)) = (&self.clipboard, self.paste_at[0].trim().parse::<f64>().ok(), self.paste_at[2].trim().parse::<f64>().ok())
            && self.tool == Tool::Clipboard
        {
            let (x1, z1) = (x + v.size[0] as f64, z + v.size[2] as f64);
            outlines.push(Outline { points: vec![(x, z), (x1, z), (x1, z1), (x, z1)], closed: true, active: false, handles: Vec::new() });
        }
        let caption = match self.hover {
            Some((x, z)) => {
                let (bx, bz) = (x.floor() as i32, z.floor() as i32);
                let id = self.ctl.map.as_ref().and_then(|m| m.identify(bx, bz)).map(|(n, y, _)| format!("   {} · Y {y}", n.trim_start_matches("minecraft:"))).unwrap_or_default();
                format!("X {bx}  Z {bz}{id}   ·   left-drag: select   right/middle-drag: pan   wheel: zoom")
            }
            None => "left-drag: select   right/middle-drag: pan   wheel: zoom".into(),
        };
        Overlay { outlines, caption, hud: true, ..Default::default() }
    }

    pub fn view(&self) -> Element<'_, EditMsg> {
        let mode = [(false, "Map"), (true, "3D preview")].iter().fold(row![].spacing(4), |r, &(v, n)| {
            r.push(button(text(n).size(12)).padding([5, 12]).on_press(EditMsg::Preview(v)).style(style::button_style(if v == self.preview.on { Btn::ChipOn } else { Btn::Chip })))
        });
        let mode = container(mode).padding(10);
        if self.preview.on {
            let status = if self.preview.frame.is_none() || self.preview.building { "loading blocks around the selection…".to_string() } else { "click two blocks to select in 3D · selection tinted blue".to_string() };
            let canvas = container(iced::widget::canvas(crate::viewer_3d::Canvas3 { frame: self.preview.frame.as_ref(), iso_pan: (0.0, 0.0), crisp: false, iso: false, mouse_look: false, locked: false, status, on_event: Box::new(EditMsg::PEv) }).width(Length::Fill).height(Length::Fill)).clip(true);
            return row![self.panel(), iced::widget::stack![canvas, mode]].height(Length::Fill).into();
        }
        let map: Element<'_, EditMsg> = match &self.ctl.map {
            Some(map) => crate::mapview::element(MapView { map, camera: self.ctl.camera, overlay: self.overlay(), on_event: Box::new(EditMsg::Map) }),
            None => container(text(if self.ctl.loading { self.ctl.status.as_str() } else { "No map" }).color(style::muted())).center(Length::Fill).into(),
        };
        row![self.panel(), iced::widget::stack![map, mode]].height(Length::Fill).into()
    }
}
