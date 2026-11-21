//! UI icons: **Pixelarticons** (Gerrit Halfmann, MIT — vendored unmodified
//! under `assets/pixelarticons/`), hand-designed on a 24-px pixel grid.
//! Each SVG is rasterized by [`crate::pxicons`] at the size asked for:
//! exact multiples of the grid stay perfectly crisp, other sizes get
//! area-coverage anti-aliasing. App / navigation icons are tinted with a
//! per-tool colour and carry a Minecraft-style drop shadow; UI glyphs are
//! one colour so the GUI can tint them per theme.

use crate::blocks::{BlockFaces, Material, iso_block};
use crate::image::{Image, Rgba, hex, shade};
use crate::pxicons;

/// Colored app / navigation icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Home,
    Worlds,
    Map,
    NbtEditor,
    Converter,
    Pruner,
    Extensions,
    Settings,
    Folder,
    Archive,
    Spawn,
    Player,
    Bookmark,
    Lighting,
    Layers,
    Star,
    Editor,
}

impl Icon {
    pub const ALL: [Icon; 17] = [
        Icon::Home,
        Icon::Worlds,
        Icon::Map,
        Icon::NbtEditor,
        Icon::Converter,
        Icon::Pruner,
        Icon::Extensions,
        Icon::Settings,
        Icon::Folder,
        Icon::Archive,
        Icon::Spawn,
        Icon::Player,
        Icon::Bookmark,
        Icon::Lighting,
        Icon::Layers,
        Icon::Star,
        Icon::Editor,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Icon::Home => "home",
            Icon::Worlds => "worlds",
            Icon::Map => "map",
            Icon::NbtEditor => "nbt-editor",
            Icon::Converter => "converter",
            Icon::Pruner => "pruner",
            Icon::Extensions => "extensions",
            Icon::Settings => "settings",
            Icon::Folder => "folder",
            Icon::Archive => "archive",
            Icon::Spawn => "spawn",
            Icon::Player => "player",
            Icon::Bookmark => "bookmark",
            Icon::Lighting => "lighting",
            Icon::Layers => "layers",
            Icon::Star => "star",
            Icon::Editor => "editor",
        }
    }
}


/// Single-color UI glyphs (tinted by the GUI theme).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Glyph {
    Search,
    Close,
    Plus,
    Grid,
    List,
    Refresh,
    Filter,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Check,
    Menu,
    Target,
    Export,
    Picture,
    // Map-rail glyphs (art grids in `map_glyphs`).
    ZoomIn,
    ZoomOut,
    Fit,
    SelBox,
    SelCircle,
    SelPolygon,
    SelSplit,
    Clock,
    ChunkGrid,
    RegionGrid,
    Spawn,
    Player,
    Bookmark,
    Entity,
    Slime,
    Cave,
    Slicer,
    Texture,
    Compass,
    Sun,
    Eye,
    Contrast,
    Layers,
    Info,
    Cube,
    Leaf,
    Palette,
    Save,
    Highlight,
}

impl Glyph {
    pub const ALL: [Glyph; 44] = [
        Glyph::Search,
        Glyph::Close,
        Glyph::Plus,
        Glyph::Grid,
        Glyph::List,
        Glyph::Refresh,
        Glyph::Filter,
        Glyph::ChevronLeft,
        Glyph::ChevronRight,
        Glyph::ChevronDown,
        Glyph::Check,
        Glyph::Menu,
        Glyph::Target,
        Glyph::Export,
        Glyph::Picture,
        Glyph::ZoomIn,
        Glyph::ZoomOut,
        Glyph::Fit,
        Glyph::SelBox,
        Glyph::SelCircle,
        Glyph::SelPolygon,
        Glyph::SelSplit,
        Glyph::Clock,
        Glyph::ChunkGrid,
        Glyph::RegionGrid,
        Glyph::Spawn,
        Glyph::Player,
        Glyph::Bookmark,
        Glyph::Entity,
        Glyph::Slime,
        Glyph::Cave,
        Glyph::Slicer,
        Glyph::Texture,
        Glyph::Compass,
        Glyph::Sun,
        Glyph::Eye,
        Glyph::Contrast,
        Glyph::Layers,
        Glyph::Info,
        Glyph::Cube,
        Glyph::Leaf,
        Glyph::Palette,
        Glyph::Save,
        Glyph::Highlight,
    ];
}


impl Icon {
    /// Pixelarticons name and tint.
    fn art(self) -> (&'static str, Rgba) {
        match self {
            Icon::Home => ("home", hex(0x8fd46a)),
            Icon::Worlds => ("earth", hex(0x5fb3e8)),
            Icon::Map => ("map", hex(0xe8c15f)),
            Icon::NbtEditor => ("braces", hex(0xb48cf0)),
            Icon::Converter => ("sort-horizontal", hex(0x5fd0c8)),
            Icon::Pruner => ("scissors", hex(0xef7a6a)),
            Icon::Extensions => ("plug", hex(0xf0a050)),
            Icon::Settings => ("gear", hex(0xc8cacf)),
            Icon::Folder => ("folder", hex(0xe8c15f)),
            Icon::Archive => ("archive", hex(0xc49a6c)),
            Icon::Spawn => ("map-pin-home", hex(0x8fd46a)),
            Icon::Player => ("user", hex(0x5fd0c8)),
            Icon::Bookmark => ("bookmark", hex(0xef7a6a)),
            Icon::Lighting => ("sun", hex(0xf5d76e)),
            Icon::Layers => ("copy", hex(0x7fa8f0)),
            Icon::Star => ("star", hex(0xf5d76e)),
            Icon::Editor => ("pencil", hex(0xf0a050)),
        }
    }
}

impl Glyph {
    /// Pixelarticons name.
    fn art(self) -> &'static str {
        match self {
            Glyph::Search => "search",
            Glyph::Close => "close",
            Glyph::Plus => "plus",
            Glyph::Grid => "grid-2x2-2",
            Glyph::List => "bulletlist",
            Glyph::Refresh => "reload",
            Glyph::Filter => "filter",
            Glyph::ChevronLeft => "chevron-left",
            Glyph::ChevronRight => "chevron-right",
            Glyph::ChevronDown => "chevron-down",
            Glyph::Check => "check",
            Glyph::Menu => "menu",
            Glyph::Target => "target",
            Glyph::Export => "upload",
            Glyph::Picture => "image",
            Glyph::ZoomIn => "zoom-in",
            Glyph::ZoomOut => "zoom-out",
            Glyph::Fit => "scale",
            Glyph::SelBox => "square-dashed-cursor",
            Glyph::SelCircle => "circle",
            Glyph::SelPolygon => "lasso",
            Glyph::SelSplit => "square-scissors",
            Glyph::Clock => "clock",
            Glyph::ChunkGrid => "grid-3x3",
            Glyph::RegionGrid => "frame",
            Glyph::Spawn => "map-pin-home",
            Glyph::Player => "user",
            Glyph::Bookmark => "bookmark",
            Glyph::Entity => "dog",
            Glyph::Slime => "potion",
            Glyph::Cave => "lightbulb-off",
            Glyph::Slicer => "sliders-vertical",
            Glyph::Texture => "blocks",
            Glyph::Compass => "compass",
            Glyph::Sun => "sun",
            Glyph::Eye => "eye",
            Glyph::Contrast => "invert",
            Glyph::Layers => "copy",
            Glyph::Info => "circle-info",
            Glyph::Cube => "box",
            Glyph::Leaf => "leaf",
            Glyph::Palette => "colors-swatch",
            Glyph::Save => "save",
            Glyph::Highlight => "sparkles",
        }
    }
}

/// A colored app icon, `px` pixels square, with a 1-grid-cell drop shadow
/// (Minecraft GUI style).
pub fn icon(i: Icon, px: u32) -> Image {
    let (name, color) = i.art();
    pxicons::render(name, px, color, Some(shade(color, 0.28)))
}

/// A single-color UI glyph, `px` pixels square.
pub fn glyph(g: Glyph, color: Rgba, px: u32) -> Image {
    pxicons::render(g.art(), px, color, None)
}

/// A large isometric block for a tool card (`px` = 2 × face size).
pub fn tool_block(m: Material, px: u32) -> Image {
    iso_block(&BlockFaces::of(m), px / 2)
}

#[cfg(test)]
#[path = "icons_tests.rs"]
mod tests;
