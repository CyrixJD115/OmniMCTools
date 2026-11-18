//! Map configuration: quality presets, detail bias, the toolbar, memory
//! budget and the on-disk tile cache location (shared by every map).

use std::sync::RwLock;

use omt_render::TileCache;
use serde::{Deserialize, Serialize};

/// Map quality presets: the chunk-read budget per tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MapQuality {
    /// Balanced, dropping to Low automatically when memory runs short.
    #[default]
    Auto,
    /// ≤ 1,024 chunk reads per tile — fastest, blurry when zoomed far out.
    Low,
    /// ≤ 4,096 chunk reads per tile.
    Balanced,
    /// ≤ 16,384 chunk reads per tile.
    High,
    /// Every chunk, at every zoom level (exact; slow on huge worlds).
    Full,
}

impl MapQuality {
    pub const ALL: [MapQuality; 5] = [Self::Auto, Self::Low, Self::Balanced, Self::High, Self::Full];

    /// Chunk reads per tile (`None` = unlimited).
    pub fn budget(self, pressure: Pressure) -> Option<u64> {
        match self {
            Self::Auto if pressure != Pressure::Normal => Some(1024),
            Self::Auto | Self::Balanced => Some(4096),
            Self::Low => Some(1024),
            Self::High => Some(16_384),
            Self::Full => None,
        }
    }
}

impl std::fmt::Display for MapQuality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "Auto",
            Self::Low => "Low (fastest)",
            Self::Balanced => "Balanced",
            Self::High => "High",
            Self::Full => "Full (exact)",
        })
    }
}

/// Detail bias choices shown in the map toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetailBias(pub i8);

impl DetailBias {
    pub const ALL: [DetailBias; 5] = [DetailBias(-2), DetailBias(-1), DetailBias(0), DetailBias(1), DetailBias(2)];
}

impl std::fmt::Display for DetailBias {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            i8::MIN..=-2 => "Fastest (−2)",
            -1 => "Faster (−1)",
            0 => "Normal",
            1 => "Sharper (+1)",
            _ => "Sharpest (+2)",
        })
    }
}

/// Map toolbar changes (Quality / Detail / Smooth). The app applies them to
/// the saved settings; every open map picks them up on its next refill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapUiMsg {
    Quality(MapQuality),
    Detail(i8),
    Smooth(bool),
}

/// The map toolbar: quality preset, detail bias, smooth scaling — the same
/// settings as Settings → Map Rendering, adjustable while looking at the map.
pub fn controls<'a, M: Clone + 'a>(on: impl Fn(MapUiMsg) -> M + Clone + 'a) -> iced::Element<'a, M> {
    use iced::widget::{pick_list, row, text, toggler};
    let c = config();
    let (on1, on2, on3) = (on.clone(), on.clone(), on);
    row![
        text("Quality").size(12),
        pick_list(MapQuality::ALL.to_vec(), Some(c.quality), move |q| on1(MapUiMsg::Quality(q))).text_size(12).width(130),
        text("Detail").size(12),
        pick_list(DetailBias::ALL.to_vec(), Some(DetailBias(c.detail_bias)), move |d: DetailBias| on2(MapUiMsg::Detail(d.0))).text_size(12).width(130),
        toggler(c.smooth).label("Smooth").size(14).text_size(12).on_toggle(move |v| on3(MapUiMsg::Smooth(v))),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Runtime map settings (mirrors the Settings page; see [`set_config`]).
#[derive(Debug, Clone, PartialEq)]
pub struct MapConfig {
    pub quality: MapQuality,
    /// In-memory tile cache cap in MB (0 = auto: 1/16 of RAM, 128–1024 MB).
    pub memory_mb: u32,
    pub disk_cache: bool,
    pub disk_cache_mb: u32,
    /// Smooth (linear) scaling for reduced-detail tiles.
    pub smooth: bool,
    /// Concurrent tile builds (0 = auto).
    pub parallel: u8,
    /// Detail bias: +1 = one level sharper than the zoom needs (more reads),
    /// -1 = one level coarser (faster).
    pub detail_bias: i8,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self { quality: MapQuality::Auto, memory_mb: 0, disk_cache: true, disk_cache_mb: 1024, smooth: true, parallel: 0, detail_bias: 0 }
    }
}

static CONFIG: RwLock<Option<MapConfig>> = RwLock::new(None);

/// Publish the map settings (startup + every Settings save).
pub fn set_config(c: MapConfig) {
    *CONFIG.write().expect("map config") = Some(c);
}

pub fn config() -> MapConfig {
    CONFIG.read().expect("map config").clone().unwrap_or_default()
}

/// The on-disk tile cache directory (`OMT_CACHE_DIR` overrides).
pub fn cache_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("OMT_CACHE_DIR") {
        return std::path::PathBuf::from(d).join("tiles");
    }
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("omt").join("tiles")
}

pub fn disk_cache(c: &MapConfig) -> Option<TileCache> {
    c.disk_cache.then(|| TileCache::new(cache_dir(), c.disk_cache_mb.max(16) as u64 * 1024 * 1024))
}

/// System memory pressure (drives cache shrinking and build pausing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    Normal,
    Low,
    Critical,
}

pub fn pressure() -> (Pressure, Option<omt_world::sysmem::MemInfo>) {
    let Some(m) = omt_world::sysmem::mem_info() else { return (Pressure::Normal, None) };
    const MB: u64 = 1024 * 1024;
    let p = if m.available < (m.total / 16).max(400 * MB) {
        Pressure::Critical
    } else if m.available < (m.total / 8).max(1024 * MB) {
        Pressure::Low
    } else {
        Pressure::Normal
    };
    (p, Some(m))
}

/// Byte budget of the in-memory tile cache under the current pressure.
pub fn memory_budget(c: &MapConfig, p: Pressure, mem: Option<omt_world::sysmem::MemInfo>) -> usize {
    const MB: usize = 1024 * 1024;
    let base = if c.memory_mb > 0 {
        c.memory_mb as usize * MB
    } else {
        mem.map_or(256 * MB, |m| ((m.total / 16) as usize).clamp(128 * MB, 1024 * MB))
    };
    match p {
        Pressure::Normal => base,
        Pressure::Low => base / 2,
        Pressure::Critical => (base / 4).max(32 * MB),
    }
}

