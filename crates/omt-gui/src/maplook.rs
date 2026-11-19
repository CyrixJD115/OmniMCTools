//! Shared "map look" for the Pruner (the Map Viewer keeps its fuller
//! sidebar): map style with sun lighting (instant post-process), chunk /
//! region grids, rulers, scale bar + compass and the spawn pin — the
//! uNmINeD-style viewing features that make sense while selecting chunks.

use omt_render::style::{MapStyle, StyleParams};
use serde::{Deserialize, Serialize};

use crate::mapview_overlay::Ruler;

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
}
