//! Small building blocks shared by the Map Viewer's side-panel tabs, so
//! every tab uses the same heading, key/value row, switch, search box and
//! list item (one look across Level info, Players, Biomes, Filters…).

use iced::widget::{button, column, container, row, space, text, text_input, toggler};
use iced::{Alignment, Color, Element, Length};

use crate::style::{self, Btn};

/// Small caps section heading.
pub fn heading<'a, M: 'a>(title: &'a str) -> Element<'a, M> {
    text(title).size(10).font(style::BOLD).color(style::hint()).into()
}

/// Muted helper text.
pub fn note<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text(s.into()).size(11).color(style::muted()).into()
}

/// `label ……… value` row.
pub fn kv<'a, M: 'a>(label: impl Into<String>, value: impl Into<String>) -> Element<'a, M> {
    row![text(label.into()).size(12).color(style::muted()).width(Length::FillPortion(2)), text(value.into()).size(12).width(Length::FillPortion(3))]
        .spacing(6)
        .align_y(Alignment::Start)
        .into()
}

/// Label + toggler on one row.
pub fn switch<'a, M: Clone + 'a>(label: impl Into<String>, on: bool, f: impl Fn(bool) -> M + 'a) -> Element<'a, M> {
    row![text(label.into()).size(12).width(Length::Fill), toggler(on).size(16).on_toggle(f)].align_y(Alignment::Center).into()
}

/// A search box.
pub fn search<'a, M: Clone + 'a>(placeholder: &'a str, value: &'a str, f: impl Fn(String) -> M + 'a) -> Element<'a, M> {
    text_input(placeholder, value).on_input(f).size(12).padding([5, 8]).style(style::input()).into()
}

/// A small color swatch.
pub fn swatch<'a, M: 'a>(rgb: [u8; 3]) -> Element<'a, M> {
    let c = Color::from_rgb8(rgb[0], rgb[1], rgb[2]);
    container(space::horizontal().width(0))
        .width(12)
        .height(12)
        .style(move |_| container::Style { background: Some(c.into()), border: iced::border::rounded(2).color(Color::BLACK.scale_alpha(0.4)).width(1), ..Default::default() })
        .into()
}

/// A full-width list row (selected rows use the active nav style).
pub fn item<'a, M: Clone + 'a>(content: impl Into<Element<'a, M>>, selected: bool, msg: M) -> Element<'a, M> {
    button(content).width(Length::Fill).padding([5, 8]).on_press(msg).style(style::button_style(if selected { Btn::NavActive } else { Btn::Nav })).into()
}

/// A compact standard button.
pub fn action<'a, M: Clone + 'a>(label: &'a str, msg: Option<M>) -> Element<'a, M> {
    button(text(label).size(12)).padding([5, 10]).on_press_maybe(msg).style(style::button_style(Btn::Standard)).into()
}

/// Two-line list text: primary + muted secondary.
pub fn two_line<'a, M: 'a>(primary: impl Into<String>, secondary: impl Into<String>) -> Element<'a, M> {
    column![text(primary.into()).size(12), text(secondary.into()).size(10).color(style::muted())].spacing(1).into()
}

/// Parse `#rrggbb` / `rrggbb` / `rgb(r,g,b)`.
pub fn parse_color(s: &str) -> Option<[u8; 3]> {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let v: Vec<u8> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        return (v.len() == 3).then(|| [v[0], v[1], v[2]]);
    }
    let h = s.trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// `#rrggbb`.
pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    #[test]
    fn colors_parse() {
        assert_eq!(super::parse_color("#ff8000"), Some([255, 128, 0]));
        assert_eq!(super::parse_color("rgb(1, 2,3)"), Some([1, 2, 3]));
        assert_eq!(super::parse_color("nope"), None);
        assert_eq!(super::hex([255, 128, 0]), "#ff8000");
    }
}
