//! Shared Photoshop-style tool rail for the map screens (Map Viewer and
//! Pruner): one button size, one active style, one tooltip placement, so
//! both apps read as the same toolbar. Tools belong to the app that owns
//! the rail; this module only fixes the geometry and the look.

use iced::widget::{button as iced_button, column, container, rule, scrollable, text, tooltip};
use iced::{Alignment, Element, Length};

use crate::style::{self, Btn};

/// Width of the rail strip (the dark column the buttons sit in).
pub const STRIP: f32 = 40.0;
/// Edge of a rail button (square, so rows stay aligned).
const BTN: f32 = 32.0;
/// Size of the content box inside a button (fixed height = uniform rows).
const BOX_W: f32 = 24.0;
const BOX_H: f32 = 20.0;

/// A rail button: one size, one active style, one tooltip. `on` lights it
/// up (chip style); tooltips point `side` (Right for a left rail, Left for
/// a right-docked strip).
pub fn button<'a, M: Clone + 'a>(
    content: impl Into<Element<'a, M>>,
    tip_label: impl Into<String>,
    on: bool,
    msg: M,
    side: tooltip::Position,
) -> Element<'a, M> {
    let b = iced_button(
        container(content.into())
            .width(BOX_W)
            .height(BOX_H)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
    )
    .width(BTN)
    .padding(4)
    .on_press(msg)
    .style(style::button_style(if on { Btn::ChipOn } else { Btn::Ghost }));
    tooltip(b, container(text(tip_label.into()).size(12)).padding([4, 8]).style(style::card()), side).into()
}

/// A symbol glyph for a rail button (`+`, `⛶`, `▭`, `⏱`…).
pub fn glyph<'a, M: 'a>(label: &'a str) -> Element<'a, M> {
    text(label).size(13).center().into()
}

/// A one-letter toggle (`E`, `S`, `C`, `X`, `T`): bold so it reads as a
/// glyph rather than a text label.
pub fn letter<'a, M: 'a>(label: &'a str) -> Element<'a, M> {
    text(label).size(12).font(style::BOLD).center().into()
}

/// Thin separator between tool groups.
pub fn separator<'a, M: 'a>() -> Element<'a, M> {
    container(rule::horizontal(1))
        .width(24)
        .align_x(Alignment::Center)
        .into()
}

/// The rail strip itself: a dark, scrollable column that centers its
/// buttons and keeps the app's rail widths identical.
pub fn strip<'a, M: Clone + 'a>(buttons: Vec<Element<'a, M>>) -> Element<'a, M> {
    let mut col = column![].spacing(2).width(BTN).align_x(Alignment::Center);
    for b in buttons {
        col = col.push(b);
    }
    container(scrollable(col).width(Length::Shrink))
        .padding([4, 0])
        .width(STRIP)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .style(style::rail())
        .into()
}
