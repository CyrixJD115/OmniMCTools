//! Map Viewer **Block filters** and **Stylesheet** tabs (uNmINeD parity):
//! the user's choices persist in [`MapRules`] and compile into
//! [`omt_render::rules::Rules`], whose id goes into the render options
//! (tiles re-render; the disk cache keys on it).

use iced::widget::{column, pick_list, row, slider, text, text_input};
use iced::{Alignment, Element, Length};
use omt_render::rules::{Category, Preset, Rules, Tints, UnknownAs};
use serde::{Deserialize, Serialize};

use crate::panelui::{self, heading, note, switch};
use crate::style;

/// Persisted filter + stylesheet choices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MapRules {
    /// Master switch of the block filter.
    pub filter_on: bool,
    pub presets: Vec<Preset>,
    /// Custom filter: ids, `*` wildcards, separated by commas/spaces.
    pub custom: String,
    pub tints: Tints,
    pub colors: Vec<(String, [u8; 3])>,
    pub unknown: UnknownAs,
    pub water_visibility: u8,
    /// Colour categories switched off (rendered plain).
    pub plain: Vec<Category>,
}

impl Default for MapRules {
    fn default() -> Self {
        Self { filter_on: true, presets: Vec::new(), custom: String::new(), tints: Tints::default(), colors: Vec::new(), unknown: UnknownAs::default(), water_visibility: omt_render::rules::WATER_VISIBILITY, plain: Vec::new() }
    }
}

impl MapRules {
    pub fn compile(&self) -> Rules {
        let mut hide = Vec::new();
        if self.filter_on {
            for p in &self.presets {
                hide.extend(p.patterns().iter().map(|s| s.to_string()));
            }
            hide.extend(omt_render::rules::parse_list(&self.custom));
        }
        hide.sort();
        hide.dedup();
        let mut plain = self.plain.clone();
        plain.sort_by_key(|c| *c as u8);
        Rules { hide, colors: self.colors.clone(), tints: self.tints, unknown: self.unknown, water_visibility: self.water_visibility, plain }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RulesMsg {
    FilterOn(bool),
    Preset(Preset, bool),
    /// Custom filter text (applied with `ApplyCustom` / Enter).
    Custom(String),
    ApplyCustom,
    TintAll(bool),
    Tint(u8, bool),
    ColorName(String),
    ColorHex(String),
    AddColor,
    RemoveColor(usize),
    Unknown(UnknownAs),
    Water(u8),
    /// Colour category on (coloured) / off (plain).
    Category(Category, bool),
    /// Read block colours from the clipboard (handled by the viewer).
    PasteColors,
    /// Clipboard text: `block = #rrggbb` lines (uNmINeD custom.blockstyles.txt).
    Pasted(Option<String>),
    Reset,
}

/// Draft fields of the tabs (not persisted).
#[derive(Debug, Clone, Default)]
pub struct RulesDraft {
    pub custom: String,
    pub color_name: String,
    pub color_hex: String,
    pub error: String,
}

/// Apply a message; returns `true` when the compiled rules changed.
pub fn apply(r: &mut MapRules, d: &mut RulesDraft, m: RulesMsg) -> bool {
    let before = r.compile();
    d.error.clear();
    match m {
        RulesMsg::FilterOn(v) => r.filter_on = v,
        RulesMsg::Preset(p, on) => {
            r.presets.retain(|x| *x != p);
            if on {
                r.presets.push(p);
            }
        }
        RulesMsg::Custom(s) => d.custom = s,
        RulesMsg::ApplyCustom => r.custom = d.custom.trim().to_string(),
        RulesMsg::TintAll(v) => r.tints = Tints { grass: v, foliage: v, water: v },
        RulesMsg::Tint(k, v) => match k {
            0 => r.tints.grass = v,
            1 => r.tints.foliage = v,
            _ => r.tints.water = v,
        },
        RulesMsg::ColorName(s) => d.color_name = s,
        RulesMsg::ColorHex(s) => d.color_hex = s,
        RulesMsg::AddColor => match (d.color_name.trim(), panelui::parse_color(&d.color_hex)) {
            ("", _) => d.error = "enter a block id or pattern (e.g. stone, *_wool)".into(),
            (_, None) => d.error = "enter a color as #rrggbb or rgb(r, g, b)".into(),
            (name, Some(c)) => {
                let name = name.to_string();
                r.colors.retain(|(n, _)| *n != name);
                r.colors.push((name, c));
                d.color_name.clear();
                d.color_hex.clear();
            }
        },
        RulesMsg::RemoveColor(i) => {
            if i < r.colors.len() {
                r.colors.remove(i);
            }
        }
        RulesMsg::Unknown(u) => r.unknown = u,
        RulesMsg::Water(w) => r.water_visibility = w,
        RulesMsg::PasteColors => {}
        RulesMsg::Pasted(text) => {
            let parsed = parse_block_styles(text.as_deref().unwrap_or(""));
            if parsed.is_empty() {
                d.error = "clipboard has no `block = #rrggbb` lines".into();
            }
            for (name, c) in parsed {
                r.colors.retain(|(n, _)| *n != name);
                r.colors.push((name, c));
            }
        }
        RulesMsg::Category(c, colored) => {
            r.plain.retain(|x| *x != c);
            if !colored {
                r.plain.push(c);
            }
        }
        RulesMsg::Reset => {
            let keep = r.filter_on;
            *r = MapRules { filter_on: keep, ..MapRules::default() };
        }
    }
    r.compile() != before
}

/// The **Block filters** tab.
pub fn filter_tab<'a, M: Clone + 'a>(r: &'a MapRules, d: &'a RulesDraft, on: impl Fn(RulesMsg) -> M + Clone + 'a) -> Element<'a, M> {
    let mut col = column![
        switch("Block filter enabled", r.filter_on, { let on = on.clone(); move |v| on(RulesMsg::FilterOn(v)) }),
        note("Filtered blocks render as air (waterlogged ones as water), so you can see what is under them."),
        heading("DEFAULT FILTERS"),
    ]
    .spacing(8);
    for p in Preset::ALL {
        let on = on.clone();
        col = col.push(switch(p.name(), r.presets.contains(&p), move |v| on(RulesMsg::Preset(p, v))));
    }
    let o1 = on.clone();
    let o2 = on.clone();
    col = col.push(heading("CUSTOM FILTER")).push(
        row![
            text_input("Enter block IDs or patterns to filter out", &d.custom).on_input(move |s| o1(RulesMsg::Custom(s))).on_submit(o2(RulesMsg::ApplyCustom)).size(12).style(style::input()).width(Length::Fill),
            panelui::action("Apply", Some(on(RulesMsg::ApplyCustom))),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    );
    col = col.push(note("e.g. stone, *_ore, minecraft:dirt — separate with commas or spaces; * matches anything."));
    if !r.custom.is_empty() {
        col = col.push(panelui::kv("Active", r.custom.clone()));
    }
    col.into()
}

/// The **Stylesheet** tab (biome tints, custom block colors, unknown
/// blocks, water).
pub fn stylesheet_tab<'a, M: Clone + 'a>(r: &'a MapRules, d: &'a RulesDraft, on: impl Fn(RulesMsg) -> M + Clone + 'a) -> Element<'a, M> {
    let t = r.tints;
    let all = t.grass || t.foliage || t.water;
    let tint = |k: u8, label: &'a str, v: bool| {
        let on = on.clone();
        switch(label, v, move |b| on(RulesMsg::Tint(k, b)))
    };
    let mut col = column![
        heading("BIOME TINTS"),
        switch("Enabled", all, { let on = on.clone(); move |v| on(RulesMsg::TintAll(v)) }),
        tint(0, "Grass", t.grass),
        tint(1, "Foliage", t.foliage),
        tint(2, "Water", t.water),
        heading("COLORS"),
        Category::ALL.iter().fold(column![].spacing(8), |col, &c| {
            let on = on.clone();
            col.push(switch(c.name(), !r.plain.contains(&c), move |v| on(RulesMsg::Category(c, v))))
        }),
        heading("WATER"),
        row![
            text("Underwater visibility").size(12).width(Length::Fill),
            text(if r.water_visibility == 0 { "flat".to_string() } else { format!("{} blocks", r.water_visibility) }).size(12),
        ],
        slider(0..=48u8, r.water_visibility, { let on = on.clone(); move |v| on(RulesMsg::Water(v)) }),
        note("0 draws flat opaque water (hides underwater structures)."),
        heading("UNKNOWN BLOCKS"),
        row![
            text("Render unknown blocks as").size(12).width(Length::Fill),
            pick_list(UnknownAs::ALL.to_vec(), Some(r.unknown), { let on = on.clone(); move |u| on(RulesMsg::Unknown(u)) }).text_size(12),
        ]
        .align_y(Alignment::Center),
        heading("CUSTOM BLOCK COLORS"),
    ]
    .spacing(8);
    for (i, (name, c)) in r.colors.iter().enumerate() {
        col = col.push(
            row![
                panelui::swatch(*c),
                text(name.clone()).size(12).width(Length::Fill),
                text(panelui::hex(*c)).size(11).color(style::muted()),
                panelui::action("✕", Some(on(RulesMsg::RemoveColor(i)))),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
    }
    let (o1, o2, o3) = (on.clone(), on.clone(), on.clone());
    col = col.push(
        row![
            text_input("block or *_pattern", &d.color_name).on_input(move |s| o1(RulesMsg::ColorName(s))).size(12).style(style::input()).width(Length::FillPortion(3)),
            text_input("#rrggbb", &d.color_hex).on_input(move |s| o2(RulesMsg::ColorHex(s))).on_submit(o3(RulesMsg::AddColor)).size(12).style(style::input()).width(Length::FillPortion(2)),
            panelui::action("Add", Some(on(RulesMsg::AddColor))),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    );
    if !d.error.is_empty() {
        col = col.push(text(d.error.clone()).size(11).color(style::warn()));
    }
    col = col.push(row![panelui::action("Paste block colors", Some(on(RulesMsg::PasteColors))), panelui::action("Reset stylesheet", Some(on(RulesMsg::Reset)))].spacing(4));
    col = col.push(note("Paste accepts `block = #rrggbb` lines (uNmINeD custom.blockstyles.txt; the Blocks tab copies this format)."));
    col.into()
}

/// Parse `block = color` lines (`//` comments; `#rrggbb` / `rgb()`).
pub fn parse_block_styles(text: &str) -> Vec<(String, [u8; 3])> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .filter_map(|l| {
            let (name, color) = l.split_once('=')?;
            Some((name.trim().to_string(), panelui::parse_color(color.trim().trim_end_matches(';'))?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_and_apply() {
        let mut r = MapRules::default();
        let mut d = RulesDraft::default();
        assert!(r.compile().is_default());
        assert!(apply(&mut r, &mut d, RulesMsg::Preset(Preset::Leaves, true)));
        assert!(r.compile().hide.contains(&"*_leaves".to_string()));
        assert!(apply(&mut r, &mut d, RulesMsg::FilterOn(false)));
        assert!(r.compile().hide.is_empty());
        d.color_name = "stone".into();
        d.color_hex = "#102030".into();
        assert!(apply(&mut r, &mut d, RulesMsg::AddColor));
        assert_eq!(r.colors, vec![("stone".to_string(), [16, 32, 48])]);
        assert!(!apply(&mut r, &mut d, RulesMsg::Custom("dirt".into())), "typing alone does not re-render");
        assert!(apply(&mut r, &mut d, RulesMsg::Pasted(Some("// c\nminecraft:grass_block = #00ff00\n*_wool = rgb(1,2,3)\nbad line".into()))));
        assert_eq!(r.colors.len(), 3);
    }
}
