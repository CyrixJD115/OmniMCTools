//! World discovery for the library: every launcher folder of this machine
//! (see [`crate::sources`]) plus the user's folders. Summaries (and folder
//! sizes) are read on a background thread so a slow disk never freezes
//! the UI; folders are scanned in parallel.

use std::path::{Path, PathBuf};

use omt_world::{Edition, WorldSummary};
use rayon::prelude::*;

use crate::settings::Settings;
use crate::sources::{Launcher, Root};

/// A discovered world plus library extras.
#[derive(Debug, Clone)]
pub struct WorldEntry {
    pub summary: WorldSummary,
    pub origin: PathBuf,
    /// Launcher + instance/profile label ("Prism · Fabric 1.21").
    pub folder_label: String,
    pub launcher: Launcher,
    pub preview: bool,
    pub last_modified: Option<std::time::SystemTime>,
    pub cheats: Option<bool>,
    /// On-disk size (bytes) of the world folder.
    pub size: u64,
    /// Version label shown on cards ("1.21.120" / "26.3 · DataVersion …").
    pub version: String,
}

impl WorldEntry {
    pub fn edition(&self) -> Edition {
        self.summary.edition
    }
}

/// Every folder to scan, de-duplicated (custom folders and the env
/// override first, then detected launcher folders that exist).
pub fn scan_roots(settings: &Settings) -> Vec<Root> {
    let mut out: Vec<Root> = Vec::new();
    if let Some(dir) = std::env::var_os("OMT_WORLDS_DIR") {
        out.push(Root { launcher: Launcher::Custom, label: "OMT_WORLDS_DIR".into(), path: PathBuf::from(dir), preview: false });
    }
    for f in &settings.world_folders {
        let label = f.file_name().map(|n| format!("Custom · {}", n.to_string_lossy())).unwrap_or_else(|| "Custom".into());
        out.push(Root { launcher: Launcher::Custom, label, path: f.clone(), preview: false });
    }
    if settings.auto_detect_folders {
        let env = |k: &str| std::env::var_os(k);
        out.extend(crate::sources::standard_roots(dirs::home_dir().as_deref(), &env));
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|r| r.path.is_dir() && seen.insert(r.path.canonicalize().unwrap_or_else(|_| r.path.clone())));
    out
}

/// Total size of a folder tree (metadata only; symlinks not followed).
pub fn dir_size(p: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                total += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

fn entry_for(path: &Path, root: &Root) -> Option<WorldEntry> {
    let detected = omt_world::detect(path).ok()?;
    let summary = WorldSummary::from_detected(&detected).ok()?;
    // Bedrock bumps `db/` on every save; level.dat is the fallback.
    let stamp = |p: PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let last_modified = [stamp(path.join("level.dat")), stamp(path.join("db")), stamp(path.join("region"))].into_iter().flatten().max();
    let cheats = match &detected {
        omt_world::Detected::BedrockFolder { level_dat, .. } => level_dat.nbt.tag.get("commandsEnabled").and_then(|t| t.as_i64()).map(|v| v != 0),
        omt_world::Detected::JavaFolder { level_dat, .. } => {
            let data = level_dat.tag.get("Data").unwrap_or(&level_dat.tag);
            data.get("allowCommands").and_then(|t| t.as_i64()).map(|v| v != 0)
        }
        omt_world::Detected::Archive(_) => None,
    };
    let version = crate::style::version_label(&summary);
    Some(WorldEntry {
        summary,
        origin: path.to_path_buf(),
        folder_label: root.label.clone(),
        launcher: root.launcher,
        preview: root.preview,
        last_modified,
        cheats,
        size: dir_size(path),
        version,
    })
}

/// Scan all roots (blocking — call from a background task).
pub fn discover(roots: &[Root]) -> Vec<WorldEntry> {
    let mut candidates: Vec<(PathBuf, &Root)> = Vec::new();
    for root in roots {
        if root.path.join("level.dat").is_file() {
            candidates.push((root.path.clone(), root));
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&root.path) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                candidates.push((p, root));
            }
        }
    }
    let mut out: Vec<WorldEntry> = candidates.par_iter().filter_map(|(p, r)| entry_for(p, r)).collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.last_modified));
    out
}

/// "1.4 GB" style sizes.
pub fn human_size(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else if v < 10.0 { format!("{v:.1} {}", U[i]) } else { format!("{v:.0} {}", U[i]) }
}

/// "3 hours ago" style relative time.
pub fn relative_time(t: std::time::SystemTime) -> String {
    let secs = std::time::SystemTime::now().duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    let (n, unit) = match secs {
        0..60 => return "just now".into(),
        60..3600 => (secs / 60, "minute"),
        3600..86_400 => (secs / 3600, "hour"),
        86_400..2_592_000 => (secs / 86_400, "day"),
        2_592_000..31_536_000 => (secs / 2_592_000, "month"),
        _ => (secs / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
        assert_eq!(human_size(200 * 1024 * 1024), "200 MB");
    }

}
