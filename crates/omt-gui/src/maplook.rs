//! Shared "map look" for the Pruner (the Map Viewer keeps its fuller
//! sidebar): map style with sun lighting (instant post-process), chunk /
//! region grids, rulers, scale bar + compass and the spawn pin — the
//! uNmINeD-style viewing features that make sense while selecting chunks.

use iced::widget::{pick_list, row, slider, text, toggler};
use iced::{Alignment, Element};
use omt_render::style::{MapStyle, StyleParams};
use serde::{Deserialize, Serialize};

use crate::mapview_overlay::Ruler;
use crate::style;

/// Remembered look of a map.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MapLook {
    pub style: StyleParams,
    /// `None` = automatic (when zoomed in).
    pub chunk_grid: Option<bool>,
    pub region_grid: bool,
    pub ruler: Ruler,
    pub hud: bool,
    pub spawn: bool,
}

impl Default for MapLook {
    fn default() -> Self {
        Self { style: StyleParams { style: MapStyle::Shaded, ..Default::default() }, chunk_grid: None, region_grid: false, ruler: Ruler::Blocks, hud: true, spawn: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LookMsg {
    Style(MapStyle),
    Azimuth(f32),
    ChunkGrid(GridChoice),
    RegionGrid(bool),
    Ruler(Ruler),
    Hud(bool),
    Spawn(bool),
}

/// Chunk-grid choices for a pick list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridChoice(pub Option<bool>);

impl GridChoice {
    pub const ALL: [GridChoice; 3] = [GridChoice(None), GridChoice(Some(true)), GridChoice(Some(false))];
}

impl std::fmt::Display for GridChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            None => "Grid: auto",
            Some(true) => "Grid: on",
            Some(false) => "Grid: off",
        })
    }
}

impl std::fmt::Display for Ruler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ruler: {}", self.name())
    }
}

impl MapLook {
    /// Apply a change; returns `true` when tiles must be re-lit.
    pub fn apply(&mut self, m: LookMsg) -> bool {
        match m {
            LookMsg::Style(s) => {
                self.style.style = s;
                return true;
            }
            LookMsg::Azimuth(a) => {
                self.style.azimuth = a;
                return true;
            }
            LookMsg::ChunkGrid(g) => self.chunk_grid = g.0,
            LookMsg::RegionGrid(v) => self.region_grid = v,
            LookMsg::Ruler(r) => self.ruler = r,
            LookMsg::Hud(v) => self.hud = v,
            LookMsg::Spawn(v) => self.spawn = v,
        }
        false
    }

    /// Compact toolbar row.
    pub fn toolbar<'a, M: Clone + 'a>(&self, on: impl Fn(LookMsg) -> M + Clone + 'a) -> Element<'a, M> {
        fn o<T, M, F: Fn(LookMsg) -> M + Clone>(on: &F, f: fn(T) -> LookMsg) -> impl Fn(T) -> M + use<T, M, F> {
            let on = on.clone();
            move |v| on(f(v))
        }
        let s = self.style;
        let mut r = row![
            pick_list(MapStyle::ALL.to_vec(), Some(s.style), o(&on, LookMsg::Style)).text_size(12).width(120),
            pick_list(GridChoice::ALL.to_vec(), Some(GridChoice(self.chunk_grid)), o(&on, LookMsg::ChunkGrid)).text_size(12).width(112),
            pick_list(Ruler::ALL.to_vec(), Some(self.ruler), o(&on, LookMsg::Ruler)).text_size(12).width(132),
            toggler(self.region_grid).label("Regions").size(14).text_size(12).on_toggle(o(&on, LookMsg::RegionGrid)),
            toggler(self.hud).label("Scale").size(14).text_size(12).on_toggle(o(&on, LookMsg::Hud)),
            toggler(self.spawn).label("Spawn").size(14).text_size(12).on_toggle(o(&on, LookMsg::Spawn)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if !matches!(s.style, MapStyle::Heightmap) {
            r = r.push(text("Sun").size(12).color(style::muted())).push(slider(0.0..=359.0, s.azimuth, o(&on, LookMsg::Azimuth)).step(5.0f32).width(110));
        }
        r.into()
    }
}
