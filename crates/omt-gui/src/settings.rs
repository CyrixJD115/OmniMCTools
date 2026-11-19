//! Persistent settings — UMT's Settings page minus everything account,
//! update-server or telemetry related (PLAN §1.2). Stored as TOML in the
//! platform config dir (`~/.config/omt/settings.toml` on Linux,
//! `%APPDATA%\omt\settings.toml` on Windows). Missing/invalid files fall
//! back to defaults; a bad file is never overwritten silently (it is moved
//! aside to `settings.toml.bad`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// UI scale (UMT: "Scale Factor", up to 200%).
    pub scale_factor: f32,
    /// Where workspaces (world copies) live; empty = platform default.
    pub working_dir: Option<PathBuf>,
    /// Extra folders scanned for worlds (Java `saves`, Bedrock `minecraftWorlds`, any folder of worlds).
    pub world_folders: Vec<PathBuf>,
    /// Also scan the standard game locations for this OS.
    pub auto_detect_folders: bool,
    /// Show friendly labels/descriptions next to raw NBT keys.
    pub friendly_names: bool,
    /// Nether default Slicer level for maps.
    pub nether_slicer: i32,
    /// Ask before pruning.
    pub confirm_prune: bool,
    /// Converter default (UMT "converter/general/options/skipProtoChunks").
    pub skip_partial_chunks: bool,
    /// Write a debug log next to the settings file.
    pub debug_log: bool,
    /// Recently opened worlds (most recent first).
    pub recent: Vec<PathBuf>,
    /// Map quality: chunk reads per map tile (Auto adapts to free memory).
    pub map_quality: crate::mapdata::MapQuality,
    /// In-memory map tile cache cap in MB (0 = automatic, 1/16 of RAM).
    pub map_memory_mb: u32,
    /// Keep rendered map tiles on disk so reopened worlds show instantly.
    pub map_disk_cache: bool,
    /// Disk tile cache size cap in MB.
    pub map_disk_cache_mb: u32,
    /// Smooth scaling for reduced-detail (sampled) map tiles.
    pub map_smooth: bool,
    /// Map tiles rendered at once (0 = automatic).
    pub map_parallel: u8,
    /// Render CPU threads (0 = all cores).
    pub map_threads: u16,
    /// Map detail bias: +1/+2 sharper (slower), -1/-2 coarser (faster).
    pub map_detail_bias: i8,
    /// Color theme.
    pub theme: crate::style::ThemeKind,
    /// Side rail collapsed to icons only.
    pub rail_collapsed: bool,
    /// World library layout.
    pub library_grid: bool,
    /// World library sort order.
    pub library_sort: crate::filter::SortBy,
    /// Starred worlds (shown first, filterable).
    pub favorites: Vec<PathBuf>,
    /// Map viewer bookmarks per world origin.
    pub bookmarks: Vec<crate::viewer::Bookmark>,
    /// Map viewer look (style, lighting, overlays) — remembered.
    pub viewer: crate::viewer::ViewerPrefs,
    /// Block textures source (installed game / download / off).
    pub texture_pack: crate::textures::TexChoice,
    /// Custom cover images per world origin (library cards).
    pub covers: Vec<(PathBuf, PathBuf)>,
    /// Pruner map look (style, grids, ruler, HUD).
    pub pruner_look: crate::maplook::MapLook,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scale_factor: 1.0,
            working_dir: None,
            world_folders: Vec::new(),
            auto_detect_folders: true,
            friendly_names: true,
            nether_slicer: 100,
            confirm_prune: true,
            skip_partial_chunks: true,
            debug_log: false,
            recent: Vec::new(),
            map_quality: crate::mapdata::MapQuality::Auto,
            map_memory_mb: 0,
            map_disk_cache: true,
            map_disk_cache_mb: 1024,
            map_smooth: true,
            map_parallel: 0,
            map_threads: 0,
            map_detail_bias: 0,
            theme: crate::style::ThemeKind::Deepslate,
            rail_collapsed: false,
            library_grid: true,
            library_sort: crate::filter::SortBy::LastPlayed,
            favorites: Vec::new(),
            bookmarks: Vec::new(),
            viewer: crate::viewer::ViewerPrefs::default(),
            texture_pack: crate::textures::TexChoice::Auto,
            covers: Vec::new(),
            pruner_look: crate::maplook::MapLook::default(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OMT_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("omt")
}

impl Settings {
    pub fn path() -> PathBuf {
        config_dir().join("settings.toml")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Self>(&text) {
                Ok(s) => s.sanitized(),
                Err(_) => {
                    let _ = std::fs::rename(&path, path.with_extension("toml.bad"));
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    fn sanitized(mut self) -> Self {
        self.scale_factor = self.scale_factor.clamp(0.5, 2.0);
        self.recent.truncate(12);
        if self.map_memory_mb != 0 {
            self.map_memory_mb = self.map_memory_mb.clamp(64, 16_384);
        }
        self.map_disk_cache_mb = self.map_disk_cache_mb.clamp(64, 65_536);
        self.map_parallel = self.map_parallel.min(8);
        self.map_detail_bias = self.map_detail_bias.clamp(-2, 2);
        self.bookmarks.truncate(500);
        self
    }

    /// Apply a map-toolbar change.
    pub fn apply_map_ui(&mut self, m: crate::mapdata::MapUiMsg) {
        match m {
            crate::mapdata::MapUiMsg::Quality(q) => self.map_quality = q,
            crate::mapdata::MapUiMsg::Detail(d) => self.map_detail_bias = d.clamp(-2, 2),
            crate::mapdata::MapUiMsg::Smooth(v) => self.map_smooth = v,
            crate::mapdata::MapUiMsg::Threads(n) => self.map_threads = n,
        }
    }

    /// The runtime map configuration these settings describe.
    pub fn map_config(&self) -> crate::mapdata::MapConfig {
        crate::mapdata::MapConfig {
            quality: self.map_quality,
            memory_mb: self.map_memory_mb,
            disk_cache: self.map_disk_cache,
            disk_cache_mb: self.map_disk_cache_mb,
            smooth: self.map_smooth,
            parallel: self.map_parallel,
            threads: self.map_threads,
            detail_bias: self.map_detail_bias,
        }
    }

    pub fn workspace_base(&self) -> PathBuf {
        self.working_dir.clone().unwrap_or_else(omt_world::workspace::default_base)
    }

    /// Custom cover of a world, when assigned and still on disk.
    pub fn cover(&self, world: &std::path::Path) -> Option<&std::path::Path> {
        self.covers.iter().find(|(w, _)| w == world).map(|(_, c)| c.as_path()).filter(|c| c.is_file())
    }

    pub fn set_cover(&mut self, world: PathBuf, image: Option<PathBuf>) {
        self.covers.retain(|(w, _)| w != &world);
        if let Some(i) = image {
            self.covers.push((world, i));
        }
    }

    pub fn is_favorite(&self, p: &std::path::Path) -> bool {
        self.favorites.iter().any(|f| f == p)
    }

    pub fn toggle_favorite(&mut self, p: PathBuf) {
        if self.is_favorite(&p) {
            self.favorites.retain(|f| f != &p);
        } else {
            self.favorites.push(p);
        }
    }

    pub fn push_recent(&mut self, path: PathBuf) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(12);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_roundtrip_and_defaults() {
        let mut s = Settings { scale_factor: 1.5, skip_partial_chunks: false, ..Default::default() };
        s.push_recent("/a".into());
        s.push_recent("/b".into());
        s.push_recent("/a".into());
        assert_eq!(s.recent, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back, s);
        // Unknown/missing keys fall back to defaults.
        let partial: Settings = toml::from_str("scale_factor = 9.0").unwrap();
        assert!(partial.skip_partial_chunks);
        assert_eq!(partial.clone().sanitized().scale_factor, 2.0);
        // Older settings files (no map keys) get map defaults; out-of-range
        // values are clamped.
        assert_eq!(partial.map_quality, crate::mapdata::MapQuality::Auto);
        assert!(partial.map_disk_cache);
        let wild: Settings = toml::from_str("map_memory_mb = 5\nmap_detail_bias = 9\nmap_quality = \"High\"").unwrap();
        let wild = wild.sanitized();
        assert_eq!((wild.map_memory_mb, wild.map_detail_bias), (64, 2));
        assert_eq!(wild.map_config().quality, crate::mapdata::MapQuality::High);
    }
}
