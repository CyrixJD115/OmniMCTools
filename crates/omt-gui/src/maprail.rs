//! Shared Photoshop-style tool rail for the map screens (Map Viewer and
//! Pruner). Both rails use the same anatomy so the two apps read as one
//! toolbar:
//!
//! 1. the app's own tools (Pruner: selection shapes + time; Viewer:
//!    markers + render modes),
//! 2. the shared **map overlays** — chunk grid, region grid, spawn,
//!    scale & compass — same glyphs, same order in both apps,
//! 3. the shared **navigation** group — zoom in, zoom out, fit — pinned to
//!    the bottom of the strip.
//!
//! Every button is a square of one size holding a 16 px monochrome
//! `omt_art` glyph; the active state is one style (`Btn::RailOn`).

use iced::widget::{button as iced_button, column, container, rule, scrollable, text, tooltip};
use iced::{Alignment, Element, Length};
use omt_art::Glyph;

use crate::assets;
use crate::style::{self, Btn};

/// Width of the rail strip (the dark column the buttons sit in).
pub const STRIP: f32 = 44.0;
/// Edge of a rail button (square, so rows and columns stay aligned).
const BTN: f32 = 36.0;
/// Glyph size: Pixelarticons are drawn on a 24-px grid; 1:1 keeps every
/// icon pixel one screen pixel.
const GLYPH: f32 = 24.0;

/// Which way tooltips point: away from the screen edge the strip is on.
#[derive(Debug, Clone, Copy)]
pub enum Side {
    /// Strip docked on the left of the map (tooltips open right).
    Left,
    /// Strip docked on the right (tooltips open left).
    Right,
}

impl Side {
    fn tip(self) -> tooltip::Position {
        match self {
            Side::Left => tooltip::Position::Right,
            Side::Right => tooltip::Position::Left,
        }
    }
}

/// A rail button: one size, one active style, one tooltip. `on` lights it
/// up; `msg = None` shows it disabled (the tooltip still explains why).
pub fn tool<'a, M: Clone + 'a>(g: Glyph, tip_label: impl Into<String>, on: bool, msg: Option<M>, side: Side) -> Element<'a, M> {
    let color = match (&msg, on) {
        (None, _) => style::hint(),
        (Some(_), true) => style::fg(),
        (Some(_), false) => style::muted(),
    };
    let b = iced_button(container(assets::glyph(g, color, GLYPH)).center(Length::Fill))
        .width(BTN)
        .height(BTN)
        .padding(0)
        .on_press_maybe(msg)
        .style(style::button_style(if on { Btn::RailOn } else { Btn::Rail }));
    tooltip(b, container(text(tip_label.into()).size(12)).padding([4, 8]).style(style::card()), side.tip()).into()
}

/// Thin separator between tool groups (same inset as the buttons).
pub fn separator<'a, M: 'a>() -> Element<'a, M> {
    container(rule::horizontal(1)).width(BTN - 8.0).padding([3, 0]).into()
}

/// State + messages of the shared map-overlay group.
pub struct Overlays<M> {
    /// `None` = automatic (shown when zoomed in).
    pub chunk_grid: Option<bool>,
    pub region_grid: bool,
    pub spawn: bool,
    /// Spawn pin applies (overworld, spawn known).
    pub spawn_available: bool,
    pub hud: bool,
    /// Mention the keyboard shortcuts in tooltips (the Viewer has them).
    pub shortcuts: bool,
    pub cycle_chunk_grid: M,
    pub toggle_region_grid: M,
    pub toggle_spawn: M,
    pub toggle_hud: M,
}

/// Chunk grid · region grid · spawn · scale & compass.
pub fn overlay_group<'a, M: Clone + 'a>(o: Overlays<M>) -> Vec<Element<'a, M>> {
    let side = Side::Left;
    let grid_tip = match o.chunk_grid {
        None => "Chunk grid: auto — shown when zoomed in",
        Some(true) => "Chunk grid: on",
        Some(false) => "Chunk grid: off",
    };
    let grid_tip = if o.shortcuts { format!("{grid_tip} (Ctrl+G)") } else { grid_tip.to_string() };
    let spawn_tip = if o.spawn_available { "World spawn pin" } else { "World spawn pin (Overworld only)" };
    vec![
        tool(Glyph::ChunkGrid, grid_tip, o.chunk_grid == Some(true), Some(o.cycle_chunk_grid), side),
        tool(Glyph::RegionGrid, "Region grid (32×32 chunks)", o.region_grid, Some(o.toggle_region_grid), side),
        tool(Glyph::Spawn, spawn_tip, o.spawn && o.spawn_available, Some(o.toggle_spawn), side),
        tool(Glyph::Compass, "Scale bar & compass", o.hud, Some(o.toggle_hud), side),
    ]
}

/// Zoom in · zoom out · fit world (pinned to the bottom of the strip).
pub fn nav_group<'a, M: Clone + 'a>(zoom_in: M, zoom_out: M, fit: M, shortcuts: bool) -> Vec<Element<'a, M>> {
    let side = Side::Left;
    let (zin, zout) = if shortcuts { ("Zoom in (+)", "Zoom out (−)") } else { ("Zoom in", "Zoom out") };
    vec![
        tool(Glyph::ZoomIn, zin, false, Some(zoom_in), side),
        tool(Glyph::ZoomOut, zout, false, Some(zoom_out), side),
        tool(Glyph::Fit, "Fit world", false, Some(fit), side),
    ]
}

/// The rail strip: `top` scrolls if the window is short, `bottom` stays
/// pinned. Buttons are centered in a strip of fixed width.
pub fn strip<'a, M: Clone + 'a>(top: Vec<Element<'a, M>>, bottom: Vec<Element<'a, M>>) -> Element<'a, M> {
    let group = |items: Vec<Element<'a, M>>| column(items).spacing(2).width(Length::Fill).align_x(Alignment::Center);
    let top = scrollable(group(top))
        .direction(scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(2).scroller_width(2)))
        .height(Length::Fill);
    let mut col = column![top].width(Length::Fill).height(Length::Fill).align_x(Alignment::Center);
    if !bottom.is_empty() {
        col = col.push(separator()).push(group(bottom));
    }
    container(col).padding([6, 0]).width(STRIP).height(Length::Fill).style(style::rail()).into()
}
