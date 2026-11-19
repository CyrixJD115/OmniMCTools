//! Map Viewer **Highlighters** (uNmINeD parity): an ordered list of
//! finders — Block finder, Biome finder, Inhabited time — each with a
//! query, a Y range, a color and a visualization, scanned over the visible
//! chunks in the background ([`omt_render::finder`]) and drawn as overlay
//! marks. "Darken map" is uNmINeD's map adjustment while highlighting.

use std::sync::Arc;

use iced::widget::{button, column, container, pick_list, row, slider, text, text_input};
use iced::{Alignment, Color, Element, Length};
use omt_render::finder::{Hit, Query};
use serde::{Deserialize, Serialize};

use crate::mapview::HlMark;
use crate::panelui::{self, heading, note, switch};
use crate::style::{self, Btn};

/// Highlighter data source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HlKind {
    Blocks,
    Biomes,
    Inhabited,
}

impl HlKind {
    pub const ALL: [HlKind; 3] = [Self::Blocks, Self::Biomes, Self::Inhabited];
}

impl std::fmt::Display for HlKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Blocks => "Block finder",
            Self::Biomes => "Biome finder",
            Self::Inhabited => "Inhabited time",
        })
    }
}

/// One highlighter (persisted).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Highlighter {
    pub enabled: bool,
    pub kind: HlKind,
    /// Block / biome ids or patterns (`*_ore`, `diamond_ore, emerald_ore`).
    pub query: String,
    pub min_y: i32,
    pub max_y: i32,
    pub color: [u8; 3],
    /// Mark opacity (0–100).
    pub opacity: u8,
    /// Inhabited time: chunks below this many minutes are skipped.
    pub min_minutes: u32,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self { enabled: true, kind: HlKind::Blocks, query: "diamond_ore, deepslate_diamond_ore".into(), min_y: -64, max_y: 320, color: [80, 230, 255], opacity: 90, min_minutes: 0 }
    }
}

impl Highlighter {
    pub fn query(&self) -> Query {
        let patterns = omt_render::rules::parse_list(&self.query);
        match self.kind {
            HlKind::Blocks => Query::Blocks { patterns, min_y: self.min_y, max_y: self.max_y },
            HlKind::Biomes => Query::Biomes { patterns, min_y: self.min_y, max_y: self.max_y },
            HlKind::Inhabited => Query::Inhabited { min_ticks: self.min_minutes as i64 * 60 * 20 },
        }
    }
}

/// Persisted highlighter settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HlPrefs {
    pub enabled: bool,
    pub items: Vec<Highlighter>,
    /// Darken the map under the marks (0–70 %).
    pub darken: u8,
}

impl Default for HlPrefs {
    fn default() -> Self {
        Self { enabled: true, items: Vec::new(), darken: 35 }
    }
}

impl HlPrefs {
    /// Active highlighters (index into `items`) with their queries.
    pub fn active(&self) -> Vec<(usize, Query)> {
        if !self.enabled {
            return Vec::new();
        }
        self.items.iter().enumerate().filter(|(_, h)| h.enabled && (h.kind == HlKind::Inhabited || !h.query.trim().is_empty())).map(|(i, h)| (i, h.query())).collect()
    }
}

#[derive(Debug, Clone)]
pub enum HlMsg {
    Enabled(bool),
    Add(HlKind),
    Remove(usize),
    Move(usize, bool),
    Toggle(usize, bool),
    Kind(usize, HlKind),
    Query(usize, String),
    MinY(usize, String),
    MaxY(usize, String),
    Color(usize, String),
    Opacity(usize, u8),
    MinMinutes(usize, u32),
    Darken(u8),
    /// Scan results for (dim, chunk rect, settings stamp).
    Done(u64, Arc<Vec<Hit>>, usize),
}

/// Scan state (not persisted).
#[derive(Debug, Default)]
pub struct HlState {
    pub hits: Arc<Vec<Hit>>,
    pub busy: bool,
    pub chunks: usize,
    /// Visible area too large to scan.
    pub too_wide: bool,
    pub color_drafts: Vec<String>,
}

/// Visible chunk count above which scanning waits for the user to zoom in.
pub const MAX_SCAN_CHUNKS: i64 = 4096;

/// Apply an edit; returns `true` when results must be recomputed.
pub fn apply(p: &mut HlPrefs, st: &mut HlState, m: HlMsg) -> bool {
    let before = p.clone();
    match m {
        HlMsg::Enabled(v) => p.enabled = v,
        HlMsg::Add(kind) => {
            let palette = [[80, 230, 255], [255, 90, 200], [255, 210, 60], [120, 255, 120], [255, 120, 60]];
            let color = palette[p.items.len() % palette.len()];
            let query = match kind {
                HlKind::Blocks => "diamond_ore, deepslate_diamond_ore",
                HlKind::Biomes => "*mushroom*",
                HlKind::Inhabited => "",
            };
            p.items.push(Highlighter { kind, color, query: query.into(), opacity: if kind == HlKind::Blocks { 90 } else { 45 }, ..Default::default() });
        }
        HlMsg::Remove(i) if i < p.items.len() => {
            p.items.remove(i);
        }
        HlMsg::Move(i, up) => {
            let j = if up { i.checked_sub(1) } else { Some(i + 1) };
            if let Some(j) = j.filter(|&j| j < p.items.len()) {
                p.items.swap(i, j);
            }
        }
        HlMsg::Toggle(i, v) => with(p, i, |h| h.enabled = v),
        HlMsg::Kind(i, k) => with(p, i, |h| h.kind = k),
        HlMsg::Query(i, s) => with(p, i, |h| h.query = s),
        HlMsg::MinY(i, s) => with(p, i, |h| h.min_y = s.trim().parse().unwrap_or(h.min_y)),
        HlMsg::MaxY(i, s) => with(p, i, |h| h.max_y = s.trim().parse().unwrap_or(h.max_y)),
        HlMsg::Color(i, s) => {
            st.color_drafts.resize(p.items.len(), String::new());
            if let Some(c) = panelui::parse_color(&s) {
                with(p, i, |h| h.color = c);
            }
            if let Some(d) = st.color_drafts.get_mut(i) {
                *d = s;
            }
        }
        HlMsg::Opacity(i, v) => with(p, i, |h| h.opacity = v),
        HlMsg::MinMinutes(i, v) => with(p, i, |h| h.min_minutes = v),
        HlMsg::Darken(v) => p.darken = v,
        HlMsg::Remove(_) | HlMsg::Done(..) => {}
    }
    // Colors, opacity and darkening only restyle the marks.
    let scan = |p: &HlPrefs| p.active().into_iter().map(|(_, q)| q).collect::<Vec<_>>();
    scan(p) != scan(&before)
}

fn with(p: &mut HlPrefs, i: usize, f: impl FnOnce(&mut Highlighter)) {
    if let Some(h) = p.items.get_mut(i) {
        f(h);
    }
}

/// Overlay marks for the hits.
pub fn marks(p: &HlPrefs, hits: &[Hit]) -> Vec<HlMark> {
    let active = p.active();
    let max_t = hits.iter().filter(|h| h.size == 16).map(|h| h.value).max().unwrap_or(1).max(1) as f64;
    hits.iter()
        .filter_map(|h| {
            let (idx, _) = active.get(h.query as usize)?;
            let hl = p.items.get(*idx)?;
            let a = hl.opacity as f32 / 100.0;
            let c = Color::from_rgba8(hl.color[0], hl.color[1], hl.color[2], a);
            Some(match hl.kind {
                // Inhabited: square grows with the chunk's time (min ¼ chunk).
                HlKind::Inhabited => {
                    let f = (h.value as f64 / max_t).sqrt().clamp(0.25, 1.0);
                    let s = 16.0 * f;
                    HlMark { x: h.x as f64 + (16.0 - s) / 2.0, z: h.z as f64 + (16.0 - s) / 2.0, size: s, color: c, dot: false }
                }
                _ => HlMark { x: h.x as f64, z: h.z as f64, size: h.size as f64, color: c, dot: h.size == 1 },
            })
        })
        .collect()
}

/// Per-highlighter hit counts.
pub fn counts(p: &HlPrefs, hits: &[Hit]) -> Vec<usize> {
    let active = p.active();
    let mut out = vec![0; p.items.len()];
    for h in hits {
        if let Some((i, _)) = active.get(h.query as usize) {
            out[*i] += 1;
        }
    }
    out
}

/// The **Highlighters** tab.
pub fn tab<'a, M: Clone + 'a>(p: &'a HlPrefs, st: &'a HlState, bedrock: bool, on: impl Fn(HlMsg) -> M + Clone + 'a) -> Element<'a, M> {
    let counts = counts(p, &st.hits);
    let o = on.clone();
    let mut col = column![switch("Highlighting enabled", p.enabled, move |v| o(HlMsg::Enabled(v)))].spacing(8);
    let status = if st.too_wide {
        "Zoom in to scan: too many chunks visible.".to_string()
    } else if st.busy {
        format!("Scanning {} chunks…", st.chunks)
    } else {
        format!("{} chunks scanned in view", st.chunks)
    };
    col = col.push(note(status));
    for (i, h) in p.items.iter().enumerate() {
        col = col.push(item(i, h, counts.get(i).copied().unwrap_or(0), st.color_drafts.get(i).map(String::as_str), bedrock, p.items.len(), on.clone()));
    }
    let add = |k: HlKind| -> Element<'a, M> { button(text(format!("+ {k}")).size(11)).padding([5, 8]).on_press(on(HlMsg::Add(k))).style(style::button_style(Btn::Standard)).into() };
    col = col.push(row![add(HlKind::Blocks), add(HlKind::Biomes), add(HlKind::Inhabited)].spacing(4));
    let o = on.clone();
    col = col.push(heading("MAP ADJUSTMENTS")).push(row![text("Darken map").size(12).width(Length::Fill), text(format!("{} %", p.darken)).size(12)]).push(slider(0..=70u8, p.darken, move |v| o(HlMsg::Darken(v))));
    col.into()
}

fn item<'a, M: Clone + 'a>(i: usize, h: &'a Highlighter, count: usize, color_draft: Option<&'a str>, bedrock: bool, n: usize, on: impl Fn(HlMsg) -> M + Clone + 'a) -> Element<'a, M> {
    let o = on.clone();
    let head = row![
        panelui::swatch(h.color),
        pick_list(HlKind::ALL.to_vec(), Some(h.kind), move |k| o(HlMsg::Kind(i, k))).text_size(12).width(Length::Fill),
        text(format!("{count}")).size(11).color(style::muted()),
        iced::widget::toggler(h.enabled).size(14).on_toggle({ let on = on.clone(); move |v| on(HlMsg::Toggle(i, v)) }),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let mut body = column![head].spacing(6);
    match h.kind {
        HlKind::Inhabited => {
            if bedrock {
                body = body.push(note("Bedrock worlds store no InhabitedTime."));
            }
            let o = on.clone();
            body = body.push(row![text("At least").size(11), text(format!("{} min", h.min_minutes)).size(11)].spacing(6)).push(slider(0..=600u32, h.min_minutes, move |v| o(HlMsg::MinMinutes(i, v))).step(5u32));
        }
        _ => {
            let (o1, o2, o3) = (on.clone(), on.clone(), on.clone());
            let what = if h.kind == HlKind::Blocks { "block ids or patterns" } else { "biome ids or patterns" };
            body = body
                .push(text_input(what, &h.query).on_input(move |s| o1(HlMsg::Query(i, s))).size(12).style(style::input()))
                .push(
                    row![
                        text("Y").size(11),
                        text_input("min", &h.min_y.to_string()).on_input(move |s| o2(HlMsg::MinY(i, s))).size(11).width(60).style(style::input()),
                        text("to").size(11),
                        text_input("max", &h.max_y.to_string()).on_input(move |s| o3(HlMsg::MaxY(i, s))).size(11).width(60).style(style::input()),
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center),
                );
        }
    }
    let (o1, o2) = (on.clone(), on.clone());
    let hex = color_draft.filter(|d| !d.is_empty()).map_or_else(|| panelui::hex(h.color), str::to_owned);
    body = body.push(
        row![
            text_input("#rrggbb", &hex).on_input(move |s| o1(HlMsg::Color(i, s))).size(11).width(78).style(style::input()),
            slider(10..=100u8, h.opacity, move |v| o2(HlMsg::Opacity(i, v))).width(Length::Fill),
            panelui::action("↑", (i > 0).then(|| on(HlMsg::Move(i, true)))),
            panelui::action("↓", (i + 1 < n).then(|| on(HlMsg::Move(i, false)))),
            panelui::action("✕", Some(on(HlMsg::Remove(i)))),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    );
    container(body).padding(8).width(Length::Fill).style(style::card()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_and_rescan_rules() {
        let (mut p, mut st) = (HlPrefs::default(), HlState::default());
        assert!(apply(&mut p, &mut st, HlMsg::Add(HlKind::Blocks)));
        assert!(!apply(&mut p, &mut st, HlMsg::Opacity(0, 40)), "styling only");
        assert!(apply(&mut p, &mut st, HlMsg::Query(0, "*_ore".into())));
        assert!(apply(&mut p, &mut st, HlMsg::Add(HlKind::Inhabited)));
        apply(&mut p, &mut st, HlMsg::Move(1, true));
        assert_eq!(p.items[0].kind, HlKind::Inhabited);
        let hits = vec![Hit { query: 1, x: 3, z: 4, size: 1, value: 12 }];
        let m = marks(&p, &hits);
        assert_eq!(m.len(), 1);
        assert!(m[0].dot);
        assert_eq!(counts(&p, &hits), vec![0, 1]);
    }
}
