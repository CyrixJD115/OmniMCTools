//! Per-block render **rules** — the map's block filter, stylesheet and
//! water settings (uNmINeD's "Block filters", "Stylesheet settings" and the
//! water part of "Render settings"):
//!
//! - `hide`: blocks drawn as air (a waterlogged hidden block becomes
//!   water), by pattern — `minecraft:oak_leaves`, `oak_leaves`, `*_leaves`,
//!   `glass*`.
//! - `colors`: custom block colors (first matching pattern wins).
//! - `tints`: biome tint per kind (grass / foliage / water).
//! - `unknown`: how blocks without a known color render.
//! - `water`: underwater visibility in blocks (0 = flat, opaque water).
//!
//! [`RenderOptions`](crate::RenderOptions) stays `Copy`: it carries only
//! the rules' content hash ([`register`]); renderers look the rules up
//! with [`get`]. The hash is part of the tile-cache key, so changing a
//! rule re-renders exactly the affected tiles.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::palette::{BlockColor, Palette, Tint};

/// Biome tint switches (uNmINeD stylesheet "Biome tints").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Tints {
    pub grass: bool,
    pub foliage: bool,
    pub water: bool,
}

impl Default for Tints {
    fn default() -> Self {
        Self { grass: true, foliage: true, water: true }
    }
}

/// Rendering of blocks with no known color (modded / future blocks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UnknownAs {
    /// A muted magenta that stands out.
    #[default]
    Highlight,
    /// Neutral stone gray.
    Gray,
    /// Skipped like air (see what is below).
    Transparent,
}

impl UnknownAs {
    pub const ALL: [UnknownAs; 3] = [Self::Highlight, Self::Gray, Self::Transparent];
    pub fn name(self) -> &'static str {
        match self {
            Self::Highlight => "Highlight (magenta)",
            Self::Gray => "Gray",
            Self::Transparent => "Transparent (skip)",
        }
    }
}

impl std::fmt::Display for UnknownAs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Stylesheet colour categories (uNmINeD "Colors"): switched off, their
/// blocks render as a plain base block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    Ores,
    Flowers,
    Coral,
    Dyed,
    Copper,
    Mycelium,
    Stones,
}

impl Category {
    pub const ALL: [Category; 7] = [Self::Ores, Self::Flowers, Self::Coral, Self::Dyed, Self::Copper, Self::Mycelium, Self::Stones];

    pub fn name(self) -> &'static str {
        match self {
            Self::Ores => "Ores",
            Self::Flowers => "Flowers",
            Self::Coral => "Coral",
            Self::Dyed => "Dyed blocks",
            Self::Copper => "Copper (weathering)",
            Self::Mycelium => "Mycelium",
            Self::Stones => "Natural stones",
        }
    }

    /// The plain block this category's block renders as (`None` = not in it).
    pub fn plain(self, name: &str) -> Option<String> {
        let id = name.strip_prefix("minecraft:").unwrap_or(name);
        let ns = |s: &str| Some(format!("minecraft:{s}"));
        match self {
            Self::Ores if id.ends_with("_ore") || id == "ancient_debris" => {
                if id.starts_with("deepslate_") {
                    ns("deepslate")
                } else if id.starts_with("nether_") || id == "ancient_debris" {
                    ns("netherrack")
                } else {
                    ns("stone")
                }
            }
            Self::Flowers if ["dandelion", "poppy", "blue_orchid", "allium", "azure_bluet", "oxeye_daisy", "cornflower", "lily_of_the_valley", "sunflower", "lilac", "rose_bush", "peony", "torchflower", "pink_petals", "wither_rose", "wildflowers"].contains(&id) || id.ends_with("_tulip") => ns("short_grass"),
            Self::Coral if id.contains("coral") && !id.starts_with("dead_") => ns(&format!("dead_{id}")),
            Self::Dyed => {
                const COLORS: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
                let rest = COLORS.iter().filter_map(|c| id.strip_prefix(c)?.strip_prefix('_')).max_by_key(|r| usize::MAX - r.len())?;
                match rest {
                    "terracotta" => ns("terracotta"),
                    "wool" | "carpet" | "concrete" | "concrete_powder" | "stained_glass" | "stained_glass_pane" | "glazed_terracotta" | "shulker_box" | "bed" | "candle" | "banner" => ns(&format!("white_{rest}")),
                    _ => None,
                }
            }
            Self::Copper if id.contains("copper") && (id.starts_with("exposed_") || id.starts_with("weathered_") || id.starts_with("oxidized_") || id.starts_with("waxed_")) => {
                let base = id.trim_start_matches("waxed_").trim_start_matches("exposed_").trim_start_matches("weathered_").trim_start_matches("oxidized_");
                ns(if base == "copper" { "copper_block" } else { base })
            }
            Self::Mycelium if id == "mycelium" => ns("dirt"),
            Self::Stones if ["granite", "diorite", "andesite", "tuff", "calcite"].contains(&id) => ns("stone"),
            _ => None,
        }
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Default underwater visibility (blocks of water blended into a column).
pub const WATER_VISIBILITY: u8 = 24;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Rules {
    pub hide: Vec<String>,
    pub colors: Vec<(String, [u8; 3])>,
    pub tints: Tints,
    pub unknown: UnknownAs,
    /// Blocks of water seen through (0 = flat opaque water surface).
    pub water_visibility: u8,
    /// Colour categories switched off (rendered plain).
    pub plain: Vec<Category>,
}

impl Default for Rules {
    fn default() -> Self {
        Self { hide: Vec::new(), colors: Vec::new(), tints: Tints::default(), unknown: UnknownAs::default(), water_visibility: WATER_VISIBILITY, plain: Vec::new() }
    }
}

/// Predefined block filters (uNmINeD's "Default filters" list; OMT's own
/// grouping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Preset {
    Water,
    Lava,
    Leaves,
    Trees,
    Plants,
    SnowIce,
    Glass,
    Torches,
}

impl Preset {
    pub const ALL: [Preset; 8] = [Self::Water, Self::Lava, Self::Leaves, Self::Trees, Self::Plants, Self::SnowIce, Self::Glass, Self::Torches];

    pub fn name(self) -> &'static str {
        match self {
            Self::Water => "Water (see the sea floor)",
            Self::Lava => "Lava",
            Self::Leaves => "Leaves",
            Self::Trees => "Trees (logs + leaves)",
            Self::Plants => "Plants, flowers & crops",
            Self::SnowIce => "Snow & ice",
            Self::Glass => "Glass",
            Self::Torches => "Torches & lights",
        }
    }

    /// Patterns this filter hides.
    pub fn patterns(self) -> &'static [&'static str] {
        match self {
            Self::Water => &["water", "bubble_column", "seagrass", "tall_seagrass", "kelp", "kelp_plant"],
            Self::Lava => &["lava"],
            Self::Leaves => &["*_leaves"],
            Self::Trees => &["*_leaves", "*_log", "*_wood", "*_stem", "*_hyphae", "mangrove_roots", "bamboo"],
            Self::Plants => &[
                "short_grass", "grass", "tall_grass", "fern", "large_fern", "dead_bush", "*_sapling", "*_flower", "*_tulip", "dandelion", "poppy", "blue_orchid", "allium",
                "azure_bluet", "oxeye_daisy", "cornflower", "lily_of_the_valley", "sunflower", "lilac", "rose_bush", "peony", "*_mushroom", "wheat", "carrots", "potatoes",
                "beetroots", "sugar_cane", "vine", "*_vines", "*_vines_plant", "sweet_berry_bush", "pink_petals", "*_roots", "*_fungus", "*_bush", "lily_pad", "cactus", "melon_stem",
                "pumpkin_stem", "pitcher_plant", "torchflower", "*_grass", "leaf_litter", "wildflowers",
            ],
            Self::SnowIce => &["snow", "snow_block", "powder_snow", "ice", "packed_ice", "blue_ice", "frosted_ice"],
            Self::Glass => &["glass", "*_glass", "glass_pane", "*_glass_pane", "tinted_glass"],
            Self::Torches => &["torch", "wall_torch", "*_torch", "*_wall_torch", "lantern", "*_lantern", "end_rod", "*_candle", "candle"],
        }
    }
}

impl std::fmt::Display for Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Split a user filter (`a, b c;d`) into patterns.
pub fn parse_list(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

/// Does `name` (`minecraft:oak_leaves`) match `pattern`? The namespace is
/// optional in the pattern; `*` matches any run of characters.
pub fn matches(pattern: &str, name: &str) -> bool {
    let (pns, pid) = pattern.split_once(':').map_or((None, pattern), |(a, b)| (Some(a), b));
    let (nns, nid) = name.split_once(':').unwrap_or(("minecraft", name));
    if pns.is_some_and(|ns| ns != "*" && ns != nns) {
        return false;
    }
    glob(pid.as_bytes(), nid.as_bytes())
}

fn glob(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si, mut star, mut mark) = (0, 0, None, 0);
    while si < s.len() {
        if pi < p.len() && (p[pi] == s[si] || p[pi] == b'?') {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

impl Rules {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Resolve a block's color under these rules.
    pub fn block(&self, palette: &Palette, state: &omt_world::BlockState) -> BlockColor {
        if self.hide.iter().any(|p| matches(p, &state.name)) {
            let waterlogged = state.properties.iter().any(|(k, v)| k == "waterlogged" && v == "true");
            return if waterlogged { palette.block(&omt_world::BlockState { name: "minecraft:water".into(), properties: Vec::new() }) } else { BlockColor::CLEAR };
        }
        if let Some((_, c)) = self.colors.iter().find(|(p, _)| matches(p, &state.name)) {
            return BlockColor { rgba: [c[0], c[1], c[2], 255], tint: Tint::None };
        }
        if let Some(plain) = self.plain.iter().find_map(|c| c.plain(&state.name)) {
            return palette.block(&omt_world::BlockState { name: plain, properties: Vec::new() });
        }
        let c = palette.block(state);
        if c == BlockColor::UNKNOWN {
            return match self.unknown {
                UnknownAs::Highlight => c,
                UnknownAs::Gray => BlockColor { rgba: [125, 125, 125, 255], tint: Tint::None },
                UnknownAs::Transparent => BlockColor::CLEAR,
            };
        }
        c
    }

    /// Whether biome tinting applies to a tint kind.
    pub fn tinted(&self, t: Tint) -> bool {
        match t {
            Tint::None => false,
            Tint::Grass => self.tints.grass,
            Tint::Foliage => self.tints.foliage,
            Tint::Water => self.tints.water,
        }
    }

    /// Stable content hash (0 = default rules).
    pub fn id(&self) -> u64 {
        if self.is_default() {
            return 0;
        }
        let json = serde_json::to_vec(self).unwrap_or_default();
        // FNV-1a: stable across runs (used in the on-disk tile cache key).
        let h = json.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
        h.max(1)
    }
}

fn registry() -> &'static RwLock<HashMap<u64, Arc<Rules>>> {
    static R: OnceLock<RwLock<HashMap<u64, Arc<Rules>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// Register rules; returns the id for [`RenderOptions::rules`](crate::RenderOptions).
pub fn register(rules: Rules) -> u64 {
    let id = rules.id();
    if id != 0 {
        registry().write().expect("rules registry").entry(id).or_insert_with(|| Arc::new(rules));
    }
    id
}

/// Rules by id (`0` or unknown → defaults).
pub fn get(id: u64) -> Arc<Rules> {
    static DEFAULT: OnceLock<Arc<Rules>> = OnceLock::new();
    if id != 0
        && let Some(r) = registry().read().expect("rules registry").get(&id)
    {
        return Arc::clone(r);
    }
    Arc::clone(DEFAULT.get_or_init(|| Arc::new(Rules::default())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(n: &str) -> omt_world::BlockState {
        omt_world::BlockState { name: n.into(), properties: vec![] }
    }

    #[test]
    fn patterns() {
        assert!(matches("*_leaves", "minecraft:oak_leaves"));
        assert!(matches("oak_leaves", "minecraft:oak_leaves"));
        assert!(matches("minecraft:glass*", "minecraft:glass_pane"));
        assert!(!matches("mod:*", "minecraft:stone"));
        assert!(!matches("leaves", "minecraft:oak_leaves"));
        assert_eq!(parse_list("water, lava;*_leaves  ice"), vec!["water", "lava", "*_leaves", "ice"]);
    }

    #[test]
    fn hide_color_and_registry() {
        let p = Palette::embedded();
        let r = Rules { hide: vec!["*_leaves".into()], colors: vec![("stone".into(), [1, 2, 3])], ..Default::default() };
        assert!(r.block(p, &st("minecraft:oak_leaves")).is_clear());
        let wl = omt_world::BlockState { name: "minecraft:oak_leaves".into(), properties: vec![("waterlogged".into(), "true".into())] };
        assert_eq!(r.block(p, &wl).tint, Tint::Water);
        assert_eq!(r.block(p, &st("minecraft:stone")).rgba, [1, 2, 3, 255]);
        assert_eq!(Rules::default().id(), 0);
        let id = register(r.clone());
        assert_ne!(id, 0);
        assert_eq!(*get(id), r);
        let plain = Rules { plain: vec![Category::Ores, Category::Dyed], ..Default::default() };
        assert_eq!(plain.block(p, &st("minecraft:deepslate_diamond_ore")), p.block(&st("minecraft:deepslate")));
        assert_eq!(plain.block(p, &st("minecraft:light_blue_wool")), p.block(&st("minecraft:white_wool")));
        assert_eq!(Category::Copper.plain("minecraft:waxed_oxidized_cut_copper").as_deref(), Some("minecraft:cut_copper"));
        assert_eq!(Category::Coral.plain("minecraft:tube_coral_block").as_deref(), Some("minecraft:dead_tube_coral_block"));
        let gray = Rules { unknown: UnknownAs::Transparent, ..Default::default() };
        assert!(gray.block(p, &st("mod:thing")).is_clear());
    }
}
