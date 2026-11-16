//! World discovery for the Home screen: standard game folders for this OS
//! plus the user's configured folders. Summaries are read in a background
//! task so a slow disk never freezes the UI.

use std::path::{Path, PathBuf};

use omt_world::WorldSummary;

use crate::settings::Settings;

/// A discovered world plus Home-card extras (UMT's world-list blurb:
/// `<Mode> Mode, Cheats` + `Last played`).
#[derive(Debug, Clone)]
pub struct WorldEntry {
    pub summary: WorldSummary,
    pub origin: PathBuf,
    pub folder_label: String,
    pub last_modified: Option<std::time::SystemTime>,
    pub cheats: Option<bool>,
}

/// Standard per-OS world folders (only those that exist are scanned).
pub fn standard_folders() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let home = dirs::home_dir();
    if let Some(h) = &home {
        out.push(("Java".to_string(), h.join(".minecraft/saves")));
        out.push(("Bedrock (mcpelauncher)".to_string(), h.join(".local/share/mcpelauncher/games/com.mojang/minecraftWorlds")));
        // Prism / MultiMC instances: every instance's saves folder.
        for base in [h.join(".local/share/PrismLauncher/instances"), h.join(".local/share/multimc/instances")] {
            if let Ok(rd) = std::fs::read_dir(&base) {
                for e in rd.flatten() {
                    let saves = e.path().join("minecraft/saves");
                    let saves = if saves.is_dir() { saves } else { e.path().join(".minecraft/saves") };
                    out.push((format!("Java ({})", e.file_name().to_string_lossy()), saves));
                }
            }
        }
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        out.push(("Java".to_string(), PathBuf::from(appdata).join(".minecraft").join("saves")));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        out.push((
            "Bedrock (Windows)".to_string(),
            PathBuf::from(local)
                .join("Packages/Microsoft.MinecraftUWP_8wekyb3d8bbwe/LocalState/games/com.mojang/minecraftWorlds"),
        ));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        // Bedrock (GDK builds, 1.21.120+) keep worlds per user id.
        let base = PathBuf::from(appdata).join("Minecraft Bedrock/Users");
        if let Ok(rd) = std::fs::read_dir(&base) {
            for e in rd.flatten() {
                out.push(("Bedrock (Windows)".to_string(), e.path().join("games/com.mojang/minecraftWorlds")));
            }
        }
    }
    out
}

/// Every folder to scan, de-duplicated.
pub fn scan_folders(settings: &Settings) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    if let Some(dir) = std::env::var_os("OMT_WORLDS_DIR") {
        out.push(("OMT_WORLDS_DIR".into(), PathBuf::from(dir)));
    }
    for f in &settings.world_folders {
        out.push(("Custom".into(), f.clone()));
    }
    if settings.auto_detect_folders {
        out.extend(standard_folders());
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|(_, p)| p.is_dir() && seen.insert(p.canonicalize().unwrap_or_else(|_| p.clone())));
    out
}

fn entry_for(path: &Path, label: &str) -> Option<WorldEntry> {
    let detected = omt_world::detect(path).ok()?;
    let summary = WorldSummary::from_detected(&detected).ok()?;
    let last_modified = std::fs::metadata(path.join("level.dat")).and_then(|m| m.modified()).ok();
    let cheats = match &detected {
        omt_world::Detected::BedrockFolder { level_dat, .. } => {
            level_dat.nbt.tag.get("commandsEnabled").and_then(|t| t.as_i64()).map(|v| v != 0)
        }
        omt_world::Detected::JavaFolder { level_dat, .. } => {
            let data = level_dat.tag.get("Data").unwrap_or(&level_dat.tag);
            data.get("allowCommands").and_then(|t| t.as_i64()).map(|v| v != 0)
        }
        omt_world::Detected::Archive(_) => None,
    };
    Some(WorldEntry { summary, origin: path.to_path_buf(), folder_label: label.to_string(), last_modified, cheats })
}

/// Scan all folders (blocking — call from a background task).
pub fn discover(folders: &[(String, PathBuf)]) -> Vec<WorldEntry> {
    let mut out = Vec::new();
    for (label, dir) in folders {
        // A folder may itself be a world.
        if let Some(e) = entry_for(dir, label) {
            out.push(e);
            continue;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir()
                && let Some(entry) = entry_for(&p, label)
            {
                out.push(entry);
            }
        }
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.last_modified));
    out
}

/// UMT-style blurb: "Survival Mode, Cheats".
pub fn blurb(e: &WorldEntry) -> String {
    let mode = match e.summary.gamemode {
        Some(0) => "Survival",
        Some(1) => "Creative",
        Some(2) => "Adventure",
        Some(3) => "Spectator",
        _ => "Unknown",
    };
    match e.cheats {
        Some(true) => format!("{mode} Mode, Cheats"),
        _ => format!("{mode} Mode"),
    }
}

/// "h:mmam, Mon d, yyyy" in local-agnostic UTC (no tz database offline).
pub fn format_time(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (h, min) = (rem / 3600, (rem % 3600) / 60);
    let (h12, ap) = match h {
        0 => (12, "am"),
        1..=11 => (h, "am"),
        12 => (12, "pm"),
        _ => (h - 12, "pm"),
    };
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{h12}:{min:02}{ap}, {} {d}, {y} UTC", MONTHS[(m - 1) as usize])
}

/// Days since 1970-01-01 → (year, month, day) (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_763_300_000);
        // 2025-11-16 13:33:20 UTC (verified with `date -u -d @1763300000`).
        assert_eq!(format_time(t), "1:33pm, Nov 16, 2025 UTC");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }
}
