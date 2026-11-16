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
        self
    }

    pub fn workspace_base(&self) -> PathBuf {
        self.working_dir.clone().unwrap_or_else(omt_world::workspace::default_base)
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
        assert_eq!(partial.sanitized().scale_factor, 2.0);
    }
}
