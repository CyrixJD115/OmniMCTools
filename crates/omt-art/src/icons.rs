//! 16×16 pixel icons. App/navigation icons are colored pixel art authored
//! as character grids (original designs); UI glyphs (search, close, grid…)
//! are drawn procedurally in one color so the GUI can tint them per theme.

use crate::blocks::{BlockFaces, Material, iso_block};
use crate::image::{Image, Rgba, hex};

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

fn palette(c: char) -> Option<Rgba> {
    Some(match c {
        'k' => hex(0x15161b),
        'w' => hex(0xf4f4f0),
        'g' => hex(0xc8cacf),
        'G' => hex(0x8c8f96),
        'd' => hex(0x55585f),
        'r' => hex(0xe5484d),
        'R' => hex(0x9c2a2e),
        'o' => hex(0xf2913d),
        'y' => hex(0xf2d27a),
        'Y' => hex(0xd9a93a),
        'l' => hex(0x7ad04f),
        'L' => hex(0x4a9a30),
        'b' => hex(0x7cc4f5),
        'B' => hex(0x3a78d8),
        'p' => hex(0xb58cf0),
        'P' => hex(0x7552b8),
        'n' => hex(0xa4744a),
        'N' => hex(0x6e4a2c),
        'c' => hex(0x5fe0e6),
        's' => hex(0xe8c4a0),
        _ => return None,
    })
}

fn grid(rows: [&str; 16]) -> Image {
    let mut img = Image::new(16, 16);
    for (y, row) in rows.iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            if let Some(c) = palette(ch) {
                img.set(x as i32, y as i32, c);
            }
        }
    }
    img
}

fn art(icon: Icon) -> [&'static str; 16] {
    match icon {
        Icon::Home => [
            "................",
            ".......kk.......",
            "......krrk......",
            ".....krrrrk.....",
            "....krrRRrrk....",
            "...krrRrrRrrk...",
            "..krrrrrrrrrrk..",
            ".kkkkkkkkkkkkkk.",
            "..kwwwwwwwwwwk..",
            "..kwnnwwwwbbwk..",
            "..kwnnwwwwbbwk..",
            "..kwnnwwwwwwwk..",
            "..kwnywwwwwwwk..",
            "..kwnnwwwwwwwk..",
            "..kkkkkkkkkkkk..",
            "................",
        ],
        Icon::Map => [
            "................",
            ".kkkkkkkkkkkkkk.",
            ".kyyyyyyyyyyyyk.",
            ".kyyLLyyyyyyyyk.",
            ".kyLLLLyyyBByyk.",
            ".kyyLLyyyBBBByk.",
            ".kyyyyynyyBByyk.",
            ".kyyyyyynyyyyyk.",
            ".kyyyynyyyyyyyk.",
            ".kyyynyyyyLLyyk.",
            ".kyyyynyyLLLLyk.",
            ".kyryrynyyLLyyk.",
            ".kyyryyyyyyyyyk.",
            ".kyryryyyyyyyyk.",
            ".kkkkkkkkkkkkkk.",
            "................",
        ],
        Icon::NbtEditor => [
            "................",
            "..kkkkkkkkkkk...",
            "..knnnnnnnnnnk..",
            "..knwwwwwwwwnk..",
            "..knwBBBBBBwnk..",
            "..knwwwwwwwwnk..",
            "..knwdddwwwwnk..",
            "..knwwwwwwwwnk..",
            "..knwddddddwnk..",
            "..knwwwwwwwwnk..",
            "..knwdddwwwwnk..",
            "..knwwwwwwwwnk..",
            "..knnnnnnnnnnk..",
            "..kNNNNNNNNNNk..",
            "...kkkkkkkkkk...",
            "................",
        ],
        Icon::Converter => [
            "................",
            "..........k.....",
            "..........kk....",
            ".kkkkkkkkkklk...",
            ".kllllllllllLk..",
            ".kLLLLLLLLLlk...",
            ".kkkkkkkkkkk....",
            "..........k.....",
            ".....k..........",
            "....kk..........",
            "...kbkkkkkkkkkk.",
            "..kBbbbbbbbbbbk.",
            "...kbBBBBBBBBBk.",
            "....kkkkkkkkkkk.",
            ".....k..........",
            "................",
        ],
        Icon::Pruner => [
            "................",
            ".kk.........kk..",
            ".kgk.......kgk..",
            "..kgk.....kgk...",
            "...kgk...kgk....",
            "....kgk.kgk.....",
            ".....kgkgk......",
            "......kdk.......",
            ".....kgkgk......",
            "....krk.krk.....",
            "...krRk.kRrk....",
            "..krk.k.k.krk...",
            "..krRRk.kRRrk...",
            "...kkk...kkk....",
            "................",
            "................",
        ],
        Icon::Extensions => [
            "................",
            ".kkkkkkkkkkkkkk.",
            ".knnnnnnnnnnnnk.",
            ".knNnnNnnNnnNnk.",
            ".kkkkkkkkkkkkkk.",
            ".kGGGGGknGGGGGk.",
            ".kGgGGGknGGgGGk.",
            ".kGGGGGknGGGGGk.",
            ".kGGgGGknGGGgGk.",
            ".kGGGGGkkGGGGGk.",
            ".kGgGGGGGGGGgGk.",
            ".kGGGGGGGGGGGGk.",
            ".kGGGgGGGGgGGGk.",
            ".kkkkkkkkkkkkkk.",
            "................",
            "................",
        ],
        Icon::Folder => [
            "................",
            "................",
            ".kkkkk..........",
            ".kYYYYk.........",
            ".kYYYYYkkkkkkkk.",
            ".kYYYYYYYYYYYYk.",
            ".kkkkkkkkkkkkkk.",
            ".kyyyyyyyyyyyyk.",
            ".kyyyyyyyyyyyyk.",
            ".kyyyyyyyyyyyyk.",
            ".kyyyyyyyyyyyyk.",
            ".kyyyyyyyyyyyyk.",
            ".kYYYYYYYYYYYYk.",
            ".kkkkkkkkkkkkkk.",
            "................",
            "................",
        ],
        Icon::Archive => [
            "................",
            "................",
            ".kkkkkkkkkkkkkk.",
            ".knnnnnnnnnnnnk.",
            ".knNNNNNNNNNNnk.",
            ".knnnnnnnnnnnnk.",
            ".kkkkkkggkkkkkk.",
            ".kNNNNNkgkNNNNk.",
            ".knnnnnkkknnnnk.",
            ".knnnnnnnnnnnnk.",
            ".knNNNNNNNNNNnk.",
            ".knnnnnnnnnnnnk.",
            ".knnnnnnnnnnnnk.",
            ".kkkkkkkkkkkkkk.",
            "................",
            "................",
        ],
        Icon::Spawn => [
            "................",
            "................",
            "................",
            "...........kkk..",
            "..........kwwwk.",
            "..........kwwwk.",
            ".kkkkkkkkkkkkkk.",
            ".krrrrrrrrrwwwk.",
            ".krRRRRRRRrwwwk.",
            ".kkkkkkkkkkkkkk.",
            ".kn..........nk.",
            ".kn..........nk.",
            "..k..........k..",
            "................",
            "................",
            "................",
        ],
        Icon::Player => [
            "................",
            "...kkkkkkkkkk...",
            "...kddddddddk...",
            "...kddddddddk...",
            "...kssssssssk...",
            "...kskssssksk...",
            "...kssssssssk...",
            "...kssskksssk...",
            "...kssssssssk...",
            "...kkkkkkkkkk...",
            "..kooooooooook..",
            ".koookooookoook.",
            ".koookooookoook.",
            ".kssk.kddk.kssk.",
            ".kkk..kddk..kkk.",
            "......kkkk......",
        ],
        Icon::Bookmark => [
            "................",
            "......kkkk......",
            ".....krrrrk.....",
            "....krrwwrrk....",
            "....krwwwwrk....",
            "....krrwwrrk....",
            ".....krrrrk.....",
            ".....krrrrk.....",
            "......krrk......",
            "......krrk......",
            ".......kk.......",
            ".......kk.......",
            "......kddk......",
            "......kddk......",
            ".......kk.......",
            "................",
        ],
        Icon::Lighting => [
            "................",
            ".......kk.......",
            ".......ky.......",
            "..kk...kk...kk..",
            "...kk......kk...",
            "......kkkk......",
            ".....kyyyyk.....",
            ".kkk.kyyyyk.kkk.",
            ".kyk.kyYYyk.kyk.",
            ".....kyyyyk.....",
            "......kkkk......",
            "...kk......kk...",
            "..kk...kk...kk..",
            ".......ky.......",
            ".......kk.......",
            "................",
        ],
        Icon::Layers => [
            "................",
            ".......kk.......",
            ".....kkbbkk.....",
            "...kkbbbbbbkk...",
            ".kkbbbbbbbbbbkk.",
            "...kkbbbbbbkk...",
            ".kk..kkbbkk..kk.",
            "...kkllkkllkk...",
            ".kkllllllllllkk.",
            "...kkllllllkk...",
            ".kk..kkllkk..kk.",
            "...kkyykkyykk...",
            ".kkyyyyyyyyyykk.",
            "...kkyyyyyykk...",
            ".....kkyykk.....",
            ".......kk.......",
        ],
        Icon::Star => [
            "................",
            ".......kk.......",
            "......kyyk......",
            "......kyyk......",
            ".....kyyyyk.....",
            "kkkkkkyyyykkkkkk",
            "kyyyyyyyyyyyyyyk",
            ".kyyyyyyyyyyyyk.",
            "..kyyyyyyyyyyk..",
            "...kyyyyyyyyk...",
            "...kyyyYYyyyk...",
            "..kyyyYkkYyyyk..",
            "..kyyYk..kYyyk..",
            ".kyYkk....kkYyk.",
            ".kkk........kkk.",
            "................",
        ],
        Icon::Editor => [
            "................",
            "....kkkkkkk.....",
            "...kcccccccck...",
            "..kckkkkkkkcck..",
            ".kck......kkcck.",
            ".kk......kknkck.",
            ".........knk.kk.",
            "........knk.....",
            ".......knk......",
            "......knk.......",
            ".....knk........",
            "....knk.........",
            "...knk..........",
            "..knk...........",
            ".kNk............",
            ".kk.............",
        ],
        // Procedural ones below never reach here.
        Icon::Worlds | Icon::Settings => ["................"; 16],
    }
}

/// A colored 16×16 icon (Worlds is a 16-px isometric grass block).
pub fn icon(i: Icon) -> Image {
    match i {
        Icon::Worlds => iso_block(&BlockFaces::grass(), 8),
        Icon::Settings => gear(hex(0xc8cacf), hex(0x15161b), hex(0x8c8f96)),
        other => grid(art(other)),
    }
}

/// A large isometric block for a tool card (`px` = 2 × face size).
pub fn tool_block(m: Material, px: u32) -> Image {
    iso_block(&BlockFaces::of(m), px / 2)
}

fn gear(fill: Rgba, outline: Rgba, hub: Rgba) -> Image {
    let mut img = Image::new(16, 16);
    let c = 7.5f32;
    for y in 0..16 {
        for x in 0..16 {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let r = (dx * dx + dy * dy).sqrt();
            let a = dy.atan2(dx);
            // 8 teeth.
            let tooth = ((a * 8.0 / std::f32::consts::TAU).rem_euclid(1.0) - 0.5).abs() < 0.22;
            let outer = if tooth { 7.6 } else { 5.6 };
            if r < 2.2 {
                continue;
            }
            if r < outer {
                img.set(x, y, if r < 3.4 { hub } else { fill });
            }
        }
    }
    outline_pass(&img, outline)
}

/// Add a 1-px dark outline around opaque pixels.
fn outline_pass(src: &Image, outline: Rgba) -> Image {
    let mut out = src.clone();
    for y in 0..src.h as i32 {
        for x in 0..src.w as i32 {
            if src.get(x, y)[3] == 0 && [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| src.get(x + dx, y + dy)[3] > 0) {
                out.set(x, y, outline);
            }
        }
    }
    // Edge pixels of the opaque area become outline too (crisper at 16 px).
    for y in 0..src.h as i32 {
        for x in 0..src.w as i32 {
            if src.get(x, y)[3] > 0 && [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| src.get(x + dx, y + dy)[3] == 0) {
                out.set(x, y, outline);
            }
        }
    }
    out
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

/// A 16×16 glyph in `color`.
pub fn glyph(g: Glyph, color: Rgba) -> Image {
    let mut img = Image::new(16, 16);
    if let Some(rows) = crate::map_glyphs::art(g) {
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                if c == '#' {
                    img.set(x as i32, y as i32, color);
                }
            }
        }
        return img;
    }
    let mut px = |x: i32, y: i32| img.set(x, y, color);
    match g {
        Glyph::Search => {
            for y in 0..16 {
                for x in 0..16 {
                    let r = ((x as f32 - 6.5).powi(2) + (y as f32 - 6.5).powi(2)).sqrt();
                    if (3.4..5.2).contains(&r) {
                        px(x, y);
                    }
                }
            }
            for i in 0..4 {
                px(10 + i, 10 + i);
                px(11 + i, 10 + i);
                px(10 + i, 11 + i);
            }
        }
        Glyph::Close => {
            for i in 3..13 {
                px(i, i);
                px(i + 1, i);
                px(15 - i, i);
                px(14 - i, i);
            }
        }
        Glyph::Plus => {
            for i in 3..13 {
                px(i, 7);
                px(i, 8);
                px(7, i);
                px(8, i);
            }
        }
        Glyph::Grid => {
            for (ox, oy) in [(2, 2), (9, 2), (2, 9), (9, 9)] {
                for y in 0..5 {
                    for x in 0..5 {
                        px(ox + x, oy + y);
                    }
                }
            }
        }
        Glyph::List | Glyph::Menu => {
            let rows: &[i32] = &[3, 7, 11];
            for &y in rows {
                let x0 = if g == Glyph::List { 6 } else { 2 };
                for x in x0..14 {
                    px(x, y);
                    px(x, y + 1);
                }
                if g == Glyph::List {
                    for d in 0..2 {
                        px(2 + d, y);
                        px(2 + d, y + 1);
                    }
                }
            }
        }
        Glyph::Refresh => {
            for y in 0..16 {
                for x in 0..16 {
                    let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
                    let r = (dx * dx + dy * dy).sqrt();
                    let a = dy.atan2(dx);
                    if (4.0..5.8).contains(&r) && !(-1.2..-0.2).contains(&a) {
                        px(x, y);
                    }
                }
            }
            for (x, y) in [(10, 1), (11, 2), (12, 3), (13, 4), (10, 4), (11, 4), (12, 4), (13, 3), (13, 2), (13, 1)] {
                px(x, y);
            }
        }
        Glyph::Filter => {
            for y in 2..9 {
                let inset = y - 2;
                for x in (1 + inset)..(15 - inset) {
                    px(x, y);
                }
            }
            for y in 9..14 {
                px(7, y);
                px(8, y);
            }
        }
        Glyph::ChevronLeft | Glyph::ChevronRight | Glyph::ChevronDown => {
            for i in 0..5 {
                let (a, b) = (4 + i, 6 + i);
                match g {
                    Glyph::ChevronLeft => {
                        px(10 - i, 3 + i);
                        px(11 - i, 3 + i);
                        px(10 - i, 12 - i);
                        px(11 - i, 12 - i);
                    }
                    Glyph::ChevronRight => {
                        px(5 + i, 3 + i);
                        px(6 + i, 3 + i);
                        px(5 + i, 12 - i);
                        px(6 + i, 12 - i);
                    }
                    _ => {
                        px(3 + i, a + 1);
                        px(3 + i, b - 1 + 1);
                        px(12 - i, a + 1);
                        px(12 - i, b - 1 + 1);
                    }
                }
            }
        }
        Glyph::Check => {
            for i in 0..4 {
                px(2 + i, 8 + i);
                px(2 + i, 9 + i);
            }
            for i in 0..8 {
                px(6 + i, 11 - i);
                px(6 + i, 10 - i);
            }
        }
        Glyph::Target => {
            for y in 0..16 {
                for x in 0..16 {
                    let r = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
                    if (5.0..6.4).contains(&r) || r < 1.6 {
                        px(x, y);
                    }
                }
            }
            for i in 0..3 {
                px(7, i);
                px(8, i);
                px(7, 15 - i);
                px(8, 15 - i);
                px(i, 7);
                px(i, 8);
                px(15 - i, 7);
                px(15 - i, 8);
            }
        }
        Glyph::Picture => {
            // Frame with a sun and a hill.
            for x in 1..15 {
                px(x, 2);
                px(x, 13);
            }
            for y in 2..14 {
                px(1, y);
                px(14, y);
            }
            for (x, y) in [(10, 5), (11, 5), (10, 6), (11, 6)] {
                px(x, y);
            }
            for i in 0..5 {
                for x in (3 + i)..(10 - i) {
                    px(x, 11 - i);
                }
                for x in (8 + i / 2)..(13 - i / 2) {
                    px(x, 11 - i / 2);
                }
            }
        }
        Glyph::Export => {
            for y in 2..11 {
                px(7, y);
                px(8, y);
            }
            for i in 0..4 {
                px(7 - i, 2 + i);
                px(8 + i, 2 + i);
            }
            for x in 2..14 {
                px(x, 13);
            }
            for y in 9..14 {
                px(2, y);
                px(13, y);
            }
        }
        // Art-grid glyphs returned above.
        _ => {}
    }
    img
}

#[cfg(test)]
#[path = "icons_tests.rs"]
mod tests;
