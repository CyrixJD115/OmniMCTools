//! Plain data types of the Map Viewer (side-panel tabs, keyboard keys,
//! remembered prefs, bookmarks, layer toggles), re-exported by `viewer`.

use std::path::PathBuf;

use omt_render::style::StyleParams;
use serde::{Deserialize, Serialize};

use crate::mapview::MapEvent;
use crate::viewer::ViewerMsg;

/// The side panel's tab (`None` = collapsed to the icon strip).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelTab {
    /// Level properties (level.dat), border, folder.
    Info,
    /// Chunk inspector (click-select a chunk).
    Inspect,
    /// Server players.
    Players,
    /// Biomes in the loaded map.
    Biomes,
    /// Surface blocks in the loaded map.
    Blocks,
    /// Entities in view.
    Entities,
    /// Block filters.
    Filter,
    /// Highlighters (block / biome finders, inhabited time).
    Highlight,
    /// Stylesheet (tints, custom colors, water, unknown blocks).
    Stylesheet,
    /// Saved map settings.
    Presets,
    /// Block + biome registry (colours, tints).
    Registry,
    /// 2D/3D and rendering quality.
    View,
    /// Map style + sun lighting + biome legend.
    Style,
    /// Grids, rulers and markers.
    Layers,
    /// Go-to, copy coordinates, bookmarks.
    Find,
    /// PNG export.
    Export,
}

impl PanelTab {
    /// Strip order: world data · look & filters · navigation & actions
    /// (the strip draws a separator between the groups).
    pub const ALL: [PanelTab; 16] = [
        Self::Info, Self::Inspect, Self::Players, Self::Biomes, Self::Blocks, Self::Entities, Self::View, Self::Style, Self::Stylesheet, Self::Filter, Self::Highlight, Self::Registry, Self::Layers, Self::Find, Self::Presets, Self::Export,
    ];

    /// Monochrome glyph for the panel's icon strip (matches the rail).
    pub const fn glyph(self) -> omt_art::Glyph {
        match self {
            Self::Info => omt_art::Glyph::Info,
            Self::Inspect => omt_art::Glyph::Cube,
            Self::Players => omt_art::Glyph::Player,
            Self::Biomes => omt_art::Glyph::Leaf,
            Self::Blocks => omt_art::Glyph::Texture,
            Self::Entities => omt_art::Glyph::Entity,
            Self::Filter => omt_art::Glyph::Filter,
            Self::Highlight => omt_art::Glyph::Highlight,
            Self::Stylesheet => omt_art::Glyph::Palette,
            Self::Presets => omt_art::Glyph::Save,
            Self::Registry => omt_art::Glyph::List,
            Self::View => omt_art::Glyph::Eye,
            Self::Style => omt_art::Glyph::Contrast,
            Self::Layers => omt_art::Glyph::Layers,
            Self::Find => omt_art::Glyph::Search,
            Self::Export => omt_art::Glyph::Export,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "Level info",
            Self::Inspect => "Chunk inspector",
            Self::Players => "Server players",
            Self::Biomes => "Biomes",
            Self::Blocks => "Blocks",
            Self::Entities => "Entities",
            Self::Filter => "Block filters",
            Self::Highlight => "Highlighters",
            Self::Stylesheet => "Stylesheet",
            Self::Presets => "Saved map settings",
            Self::Registry => "Registry",
            Self::View => "Render settings",
            Self::Style => "Map style",
            Self::Layers => "Layers",
            Self::Find => "Find",
            Self::Export => "Export",
        }
    }
}

/// Keyboard navigation (uNmINeD-style shortcuts, applied to the viewer
/// session on screen; see `handlers::handle_hotkey`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewerKey {
    /// Pan by blocks: arrows 16, Shift+arrows 1, Ctrl+arrows 256.
    Pan(f64, f64),
    ZoomIn,
    ZoomOut,
    /// Back to 1:1.
    ZoomReset,
    CycleChunkGrid,
}

impl From<ViewerKey> for ViewerMsg {
    fn from(k: ViewerKey) -> Self {
        match k {
            ViewerKey::Pan(dx, dz) => Self::Map(MapEvent::Pan { dx, dz }),
            ViewerKey::ZoomIn => Self::Zoom(2.0),
            ViewerKey::ZoomOut => Self::Zoom(0.5),
            ViewerKey::ZoomReset => Self::ZoomReset,
            ViewerKey::CycleChunkGrid => Self::CycleChunkGrid,
        }
    }
}

/// Remembered viewer look (Settings TOML).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewerPrefs {
    pub style: StyleParams,
    /// `None` = automatic.
    pub chunk_grid: Option<bool>,
    pub region_grid: bool,
    pub slime: bool,
    pub spawn: bool,
    pub players: bool,
    pub bookmarks: bool,
    pub hud: bool,
    pub biome_tint: bool,
    /// Real block textures when zoomed in (needs a texture pack).
    pub textured: bool,
    /// Entity pins when zoomed in.
    pub entities: bool,
    /// Rulers along the map edges.
    pub ruler: crate::mapview_overlay::Ruler,
    /// 3D render resolution relative to the screen's pixels (1 = native).
    pub render_scale: f32,
    /// 3D "Blocks" render distance (blocks from the camera).
    pub block_distance: i32,
    /// 3D mouse controls.
    pub controls: crate::viewer_3d_input::Controls3,
    /// 3D collision (Normal stays above ground, Spectator flies through).
    pub movement: crate::viewer_3d_input::Move3,
    /// Look sensitivity multiplier.
    pub look_sens: f32,
    /// Keyboard fly speed (blocks / second; Ctrl × 2.5).
    pub fly_speed: f32,
    /// Which side-panel tab is showing (TOML-friendly: two plain fields).
    pub panel_tab: PanelTab,
    /// The side panel is collapsed to its icon strip.
    pub panel_closed: bool,
    /// Block filters + stylesheet (render rules).
    pub rules: crate::viewer_rules::MapRules,
    pub highlights: crate::viewer_highlight::HlPrefs,
    /// World border (Java level.dat) on the map.
    pub border: bool,
    /// `r.x.z` labels on the region grid.
    pub region_labels: bool,
    pub presets: Vec<crate::viewer_presets::MapPreset>,
    /// Saved settings applied to newly opened worlds.
    pub default_preset: Option<String>,
}

impl Default for ViewerPrefs {
    fn default() -> Self {
        Self { style: StyleParams::default(), chunk_grid: None, region_grid: false, slime: false, spawn: true, players: true, bookmarks: true, hud: true, biome_tint: true, textured: true, entities: true, ruler: crate::mapview_overlay::Ruler::Blocks, render_scale: 1.0,
            block_distance: 192,
            controls: crate::viewer_3d_input::Controls3::Drag,
            movement: crate::viewer_3d_input::Move3::Normal,
            look_sens: 1.0,
            fly_speed: 20.0,
            panel_tab: PanelTab::View,
            panel_closed: false,
            rules: Default::default(),
            highlights: Default::default(),
            border: true,
            region_labels: true,
            presets: Vec::new(),
            default_preset: None,
        }
    }
}

impl ViewerPrefs {
    /// The open side-panel tab (`None` = collapsed to the icon strip).
    pub fn panel(&self) -> Option<PanelTab> {
        (!self.panel_closed).then_some(self.panel_tab)
    }

    /// Open a tab, or collapse the panel (`None`).
    pub fn set_panel(&mut self, panel: Option<PanelTab>) {
        match panel {
            Some(t) => {
                self.panel_tab = t;
                self.panel_closed = false;
            }
            None => self.panel_closed = true,
        }
    }
}

/// A saved place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub world: PathBuf,
    pub dim: u8,
    pub x: i32,
    pub z: i32,
    pub name: String,
}

/// Layer toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    RegionGrid,
    Slime,
    Spawn,
    Players,
    Bookmarks,
    Hud,
    BiomeTint,
    Textures,
    Entities,
    Border,
    RegionLabels,
}
