//! Renders an extension tool's declarative UI ([`omt_extensions::UiNode`])
//! with OMT's own widgets and theme. Containers draw as Minecraft
//! inventories: grey panel, inset slots, item icons from the active
//! textures, stack counts with a shadow, enchantment glint and the purple
//! item tooltip.

use std::collections::HashMap;

use iced::widget::{button, column, container, mouse_area, pick_list, progress_bar, row, rule, scrollable, slider, space, stack, text, text_input, toggler, tooltip};
use iced::{Alignment, Background, Border, Color, Element, Font, Length};
use omt_art::Glyph;
use omt_extensions::{ButtonStyle, ItemSlot, Pick, ToolEvent, UiNode};

use crate::Message;
use crate::assets;
use crate::ext_tools::ExtToolMsg;
use crate::style::{self, Btn};

/// What a render needs besides the tree.
pub struct Ctx<'a> {
    pub uid: u64,
    /// Values typed / dragged locally, not yet echoed by the tool.
    pub overrides: &'a HashMap<String, String>,
    /// Library worlds for `pick = "world"` buttons: (name, folder).
    pub worlds: &'a [(String, String)],
    pub busy: bool,
}

fn ev(c: &Ctx<'_>, e: ToolEvent) -> Message {
    Message::ExtTool(ExtToolMsg::Event(c.uid, e))
}

/// A UI glyph by name (extension icons); `None` = unknown.
pub fn glyph_named(name: &str) -> Option<Glyph> {
    Some(match name {
        "search" | "find" => Glyph::Search,
        "close" | "x" => Glyph::Close,
        "plus" | "add" => Glyph::Plus,
        "grid" => Glyph::Grid,
        "list" => Glyph::List,
        "refresh" | "reload" => Glyph::Refresh,
        "filter" => Glyph::Filter,
        "check" | "ok" => Glyph::Check,
        "target" => Glyph::Target,
        "export" | "upload" => Glyph::Export,
        "image" | "picture" => Glyph::Picture,
        "clock" | "time" => Glyph::Clock,
        "spawn" | "home" => Glyph::Spawn,
        "player" | "user" => Glyph::Player,
        "bookmark" => Glyph::Bookmark,
        "entity" | "mob" => Glyph::Entity,
        "slime" | "potion" => Glyph::Slime,
        "cave" => Glyph::Cave,
        "texture" | "blocks" => Glyph::Texture,
        "compass" => Glyph::Compass,
        "sun" => Glyph::Sun,
        "eye" | "view" => Glyph::Eye,
        "layers" => Glyph::Layers,
        "info" => Glyph::Info,
        "box" | "cube" | "chest" => Glyph::Cube,
        "leaf" => Glyph::Leaf,
        "palette" | "color" => Glyph::Palette,
        "save" => Glyph::Save,
        "sparkles" | "highlight" => Glyph::Highlight,
        "map" | "chunks" => Glyph::ChunkGrid,
        _ => return None,
    })
}

pub fn render<'a>(node: &'a UiNode, c: &Ctx<'a>) -> Element<'a, Message> {
    match node {
        UiNode::Column { children, spacing } => column(children.iter().map(|n| render(n, c))).spacing(*spacing).width(Length::Fill).into(),
        UiNode::Row { children, spacing } => row(children.iter().map(|n| render(n, c))).spacing(*spacing).align_y(Alignment::Center).wrap().into(),
        UiNode::Card { title, children } => {
            let mut col = column![].spacing(8);
            if let Some(t) = title {
                col = col.push(text(t.clone()).size(14).font(style::BOLD));
            }
            container(col.extend(children.iter().map(|n| render(n, c)))).padding(12).width(Length::Fill).style(style::card()).into()
        }
        UiNode::Scroll { child, height } => {
            let s = scrollable(render(child, c)).width(Length::Fill);
            match height {
                Some(h) => s.height(Length::Fixed(*h)).into(),
                None => s.height(Length::Shrink).into(),
            }
        }
        UiNode::Text { text: t, size, color, bold, mono } => {
            let mut w = text(t.clone()).size(*size);
            if let Some((r, g, b)) = color {
                w = w.color(Color::from_rgb8(*r, *g, *b));
            }
            if *bold {
                w = w.font(style::BOLD);
            }
            if *mono {
                w = w.font(Font::MONOSPACE);
            }
            w.into()
        }
        UiNode::Heading { text: t } => text(t.clone()).size(17).font(style::BOLD).into(),
        UiNode::Separator => rule::horizontal(1).into(),
        UiNode::Space { size } => space::vertical().height(*size).into(),
        UiNode::Button { id, label, style: bs, pick, enabled, icon } => {
            if *pick == Pick::World {
                let names: Vec<String> = c.worlds.iter().map(|(n, _)| n.clone()).collect();
                let (uid, id) = (c.uid, id.clone());
                let worlds = c.worlds.to_vec();
                return pick_list(names, None::<String>, move |n: String| {
                    let path = worlds.iter().find(|(w, _)| *w == n).map(|(_, p)| p.clone()).unwrap_or_default();
                    Message::ExtTool(ExtToolMsg::Event(uid, ToolEvent::Picked { id: id.clone(), path }))
                })
                .placeholder(label.clone())
                .text_size(12)
                .into();
            }
            let kind = match bs {
                ButtonStyle::Primary => Btn::Primary,
                ButtonStyle::Danger => Btn::Danger,
                ButtonStyle::Ghost => Btn::Ghost,
                ButtonStyle::Standard => Btn::Standard,
            };
            let mut content = row![].spacing(6).align_y(Alignment::Center);
            if let Some(g) = icon.as_deref().and_then(glyph_named) {
                let tint = if kind == Btn::Primary || kind == Btn::Danger { style::t().on_accent } else { style::fg() };
                content = content.push(assets::glyph(g, tint, 16.0));
            }
            content = content.push(text(label.clone()).size(12));
            let msg = match pick {
                Pick::None => ev(c, ToolEvent::Click { id: id.clone() }),
                p => Message::ExtTool(ExtToolMsg::Pick(c.uid, id.clone(), *p)),
            };
            button(content).padding([6, 12]).on_press_maybe((*enabled && !c.busy).then_some(msg)).style(style::button_style(kind)).into()
        }
        UiNode::Input { id, value, placeholder, width } => {
            let shown = c.overrides.get(id).cloned().unwrap_or_else(|| value.clone());
            let (uid, id2, id3) = (c.uid, id.clone(), id.clone());
            let w = text_input(placeholder, &shown)
                .on_input(move |v| Message::ExtTool(ExtToolMsg::Typed(uid, id2.clone(), v)))
                .on_submit(Message::ExtTool(ExtToolMsg::Submit(uid, id3)))
                .size(12)
                .padding([6, 8])
                .style(style::input());
            match width {
                Some(px) => w.width(Length::Fixed(*px)).into(),
                None => w.width(Length::Fill).into(),
            }
        }
        UiNode::Toggle { id, label, value } => {
            let (uid, id) = (c.uid, id.clone());
            toggler(*value).label(label.clone()).text_size(12).on_toggle(move |v| Message::ExtTool(ExtToolMsg::Event(uid, ToolEvent::Toggle { id: id.clone(), value: v }))).into()
        }
        UiNode::Slider { id, min, max, step, value, label } => {
            let shown = c.overrides.get(id).and_then(|v| v.parse().ok()).unwrap_or(*value);
            let (uid, a, b) = (c.uid, id.clone(), id.clone());
            let s = slider(*min..=*max, shown, move |v| Message::ExtTool(ExtToolMsg::Dragged(uid, a.clone(), v)))
                .step(*step)
                .on_release(Message::ExtTool(ExtToolMsg::Released(uid, b)));
            row![text(label.clone()).size(12), s, text(format!("{shown}")).size(12).color(style::muted())].spacing(8).align_y(Alignment::Center).into()
        }
        UiNode::Select { id, options, value, placeholder } => {
            let (uid, id) = (c.uid, id.clone());
            pick_list(options.clone(), value.clone(), move |v| Message::ExtTool(ExtToolMsg::Event(uid, ToolEvent::Select { id: id.clone(), value: v })))
                .placeholder(placeholder.clone())
                .text_size(12)
                .into()
        }
        UiNode::Table { id, columns, rows, selected, height } => table(id, columns, rows, *selected, *height, c),
        UiNode::Chest { id, title, rows, cols, items, selected } => chest(id, title, *rows, *cols, items, *selected, c),
        UiNode::Block { name, size } => assets::block_icon(name, *size),
        UiNode::Progress { value, label } => column![progress_bar(0.0..=1.0, *value).girth(8), text(label.clone()).size(11).color(style::muted())].spacing(4).into(),
        UiNode::Code { text: t } => container(scrollable(text(t.clone()).font(Font::MONOSPACE).size(12)).direction(scrollable::Direction::Both { vertical: scrollable::Scrollbar::default(), horizontal: scrollable::Scrollbar::default() }))
            .padding(10)
            .width(Length::Fill)
            .max_height(420)
            .style(style::card_flat())
            .into(),
    }
}

fn table<'a>(id: &str, columns: &'a [String], rows: &'a [Vec<String>], selected: Option<usize>, height: Option<f32>, c: &Ctx<'a>) -> Element<'a, Message> {
    let cells = |vals: Vec<Element<'a, Message>>| row(vals.into_iter().map(|e| container(e).width(Length::FillPortion(1)).into())).spacing(8);
    let header = cells(columns.iter().map(|h| text(h.clone()).size(11).font(style::BOLD).color(style::muted()).into()).collect());
    let body = column(rows.iter().enumerate().map(|(i, r)| {
        let on = selected == Some(i);
        button(cells(r.iter().map(|v| text(v.clone()).size(12).into()).collect()))
            .width(Length::Fill)
            .padding([4, 8])
            .on_press(ev(c, ToolEvent::Row { id: id.to_string(), row: i }))
            .style(style::button_style(if on { Btn::NavActive } else { Btn::Nav }))
            .into()
    }))
    .spacing(1);
    let body: Element<'a, Message> = match height {
        Some(h) => scrollable(body).height(Length::Fixed(h)).into(),
        None => body.into(),
    };
    column![container(header).padding([0, 8]), rule::horizontal(1), body].spacing(4).width(Length::Fill).into()
}

/// Minecraft inventory colours.
const PANEL: Color = Color::from_rgb(0.776, 0.776, 0.776);
const SLOT: Color = Color::from_rgb(0.545, 0.545, 0.545);
const SLOT_DARK: Color = Color::from_rgb(0.216, 0.216, 0.216);
const TITLE: Color = Color::from_rgb(0.25, 0.25, 0.25);
const SLOT_PX: f32 = 36.0;

fn tooltip_box<'a>(it: &'a ItemSlot) -> Element<'a, Message> {
    let title = it.label.clone().unwrap_or_else(|| omt_extensions::api::pretty_id(&it.id));
    let name_color = if it.label.is_some() { Color::from_rgb8(0x55, 0xff, 0xff) } else if it.glint { Color::from_rgb8(0xff, 0xff, 0x55) } else { Color::WHITE };
    let mut col = column![text(title).size(13).color(name_color)].spacing(2);
    for l in &it.lore {
        col = col.push(text(l.clone()).size(12).color(Color::from_rgb8(0xa8, 0x9c, 0xd8)));
    }
    col = col.push(text(format!("{} ×{} · slot {}", it.id, it.count, it.slot)).size(11).color(Color::from_rgb8(0x80, 0x80, 0x80)));
    container(col)
        .padding([6, 8])
        .style(|_| container::Style { background: Some(Background::Color(Color::from_rgba8(0x10, 0x00, 0x10, 0.94))), border: Border { color: Color::from_rgb8(0x28, 0x00, 0x5e), width: 2.0, radius: 2.0.into() }, ..Default::default() })
        .into()
}

fn slot<'a>(id: &str, idx: i32, it: Option<&'a ItemSlot>, selected: bool, c: &Ctx<'a>) -> Element<'a, Message> {
    let bg = container(space::horizontal()).width(SLOT_PX).height(SLOT_PX).style(move |_| container::Style {
        background: Some(Background::Color(SLOT)),
        border: Border { color: if selected { style::accent() } else { SLOT_DARK }, width: if selected { 2.0 } else { 1.0 }, radius: 0.0.into() },
        ..Default::default()
    });
    let mut layers = stack![bg];
    if let Some(it) = it {
        layers = layers.push(container(assets::item_icon(&it.id, 32.0)).center(SLOT_PX));
        if it.glint {
            layers = layers.push(container(space::horizontal()).width(SLOT_PX).height(SLOT_PX).style(|_| container::Style { background: Some(Background::Color(Color::from_rgba8(0xa0, 0x60, 0xff, 0.22))), ..Default::default() }));
        }
        if it.count != 1 {
            let n = it.count.to_string();
            let shadow = container(text(n.clone()).size(12).font(style::BOLD).color(Color::from_rgb8(0x3f, 0x3f, 0x3f))).width(SLOT_PX).height(SLOT_PX).align_right(SLOT_PX).align_bottom(SLOT_PX).padding(iced::Padding { top: 0.0, right: 1.0, bottom: 0.0, left: 0.0 });
            let fg = container(text(n).size(12).font(style::BOLD).color(Color::WHITE)).width(SLOT_PX).height(SLOT_PX).align_right(SLOT_PX).align_bottom(SLOT_PX).padding(iced::Padding { top: 0.0, right: 2.0, bottom: 1.0, left: 0.0 });
            layers = layers.push(shadow).push(fg);
        }
    }
    let area = mouse_area(layers).on_press(ev(c, ToolEvent::Slot { id: id.to_string(), slot: idx }));
    match it {
        Some(it) => tooltip(area, tooltip_box(it), tooltip::Position::FollowCursor).into(),
        None => area.into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn chest<'a>(id: &str, title: &'a str, rows: usize, cols: usize, items: &'a [ItemSlot], selected: Option<i32>, c: &Ctx<'a>) -> Element<'a, Message> {
    let by_slot: HashMap<i32, &ItemSlot> = items.iter().map(|i| (i.slot, i)).collect();
    // Items without a slot fill the first free cells in order.
    let mut loose = items.iter().filter(|i| i.slot < 0);
    let mut grid = column![].spacing(0);
    for r in 0..rows {
        let mut line = row![].spacing(0);
        for col in 0..cols {
            let idx = (r * cols + col) as i32;
            let it = by_slot.get(&idx).copied().or_else(|| if by_slot.contains_key(&idx) { None } else { loose.next() });
            line = line.push(slot(id, idx, it, selected == Some(idx), c));
        }
        grid = grid.push(line);
    }
    let mut col = column![].spacing(6);
    if !title.is_empty() {
        col = col.push(text(omt_extensions::api::pretty_id(title)).size(13).color(TITLE));
    }
    container(col.push(grid))
        .padding(8)
        .style(|_| container::Style {
            background: Some(Background::Color(PANEL)),
            border: Border { color: Color::from_rgb8(0x55, 0x55, 0x55), width: 2.0, radius: 3.0.into() },
            shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.35), offset: iced::Vector::new(0.0, 3.0), blur_radius: 6.0 },
            ..Default::default()
        })
        .into()
}
