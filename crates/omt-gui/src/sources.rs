//! Where worlds live: every launcher / install flavour OMT knows, resolved
//! to concrete "worlds folders" for this computer.
//!
//! Linux Bedrock has two very different stacks:
//! * **mcpelauncher** (Android build): data root `~/.local/share/mcpelauncher`
//!   or, as a Flatpak, `~/.var/app/io.mrarm.mcpelauncher/data/mcpelauncher`;
//!   worlds under `games/com.mojang/minecraftWorlds`. Launcher profiles in
//!   `<data>/mcpelauncher/profiles/profiles.ini` (Qt INI) may point a
//!   profile at a custom data root (`dataDirCustom=true`, `dataDir=…`).
//!   Source: minecraft-linux.github.io FAQ + mcpelauncher-ui-qt
//!   `profilemanager.cpp`.
//! * **BedrockOnLinux** (the Windows GDK build under a GDK-Proton prefix):
//!   `~/.local/share/bedrock-on-linux/compatdata/pfx/drive_c/users/steamuser/
//!   AppData/Roaming/Minecraft Bedrock` (quoted from its uninstall notes);
//!   below that the standard GDK layout `Users/<id>/games/com.mojang/
//!   minecraftWorlds`. Prefixes are discovered by search (the install can be
//!   moved and profiles get separate prefixes), Preview included.
//!
//! Windows: GDK builds (`%APPDATA%\Minecraft Bedrock[ Preview]\Users\<id>`),
//! the older UWP packages, and Java's `.minecraft`. Java on Linux: vanilla,
//! Flatpak, Prism/MultiMC instances, Modrinth profiles.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// A launcher / install flavour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Launcher {
    JavaVanilla,
    Prism,
    MultiMc,
    Modrinth,
    Mcpelauncher,
    BedrockOnLinux,
    BedrockWindows,
    BedrockUwp,
    Custom,
}

impl Launcher {
    pub const ALL: [Launcher; 9] = [
        Self::JavaVanilla,
        Self::Prism,
        Self::MultiMc,
        Self::Modrinth,
        Self::Mcpelauncher,
        Self::BedrockOnLinux,
        Self::BedrockWindows,
        Self::BedrockUwp,
        Self::Custom,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::JavaVanilla => "Java Launcher",
            Self::Prism => "Prism Launcher",
            Self::MultiMc => "MultiMC",
            Self::Modrinth => "Modrinth App",
            Self::Mcpelauncher => "mcpelauncher",
            Self::BedrockOnLinux => "BedrockOnLinux",
            Self::BedrockWindows => "Bedrock (Windows)",
            Self::BedrockUwp => "Bedrock (UWP)",
            Self::Custom => "Custom folder",
        }
    }

    /// Short token for the search syntax (`source:prism`).
    pub fn token(self) -> &'static str {
        match self {
            Self::JavaVanilla => "vanilla",
            Self::Prism => "prism",
            Self::MultiMc => "multimc",
            Self::Modrinth => "modrinth",
            Self::Mcpelauncher => "mcpelauncher",
            Self::BedrockOnLinux => "bol",
            Self::BedrockWindows => "windows",
            Self::BedrockUwp => "uwp",
            Self::Custom => "custom",
        }
    }

    pub fn from_token(s: &str) -> Option<Self> {
        let s = s.to_ascii_lowercase();
        Self::ALL.into_iter().find(|l| l.token() == s || l.name().to_ascii_lowercase().replace(' ', "") == s.replace(' ', "")).or(match s.as_str() {
            "mcpe" | "mcpel" => Some(Self::Mcpelauncher),
            "bedrockonlinux" | "proton" => Some(Self::BedrockOnLinux),
            "gdk" | "win" => Some(Self::BedrockWindows),
            "java" | "minecraft" => Some(Self::JavaVanilla),
            _ => None,
        })
    }
}

/// One folder that holds worlds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub launcher: Launcher,
    /// Human label: launcher plus instance/profile ("Prism · Fabric 1.21").
    pub label: String,
    pub path: PathBuf,
    /// Preview / beta channel.
    pub preview: bool,
}

impl Root {
    fn new(launcher: Launcher, label: impl Into<String>, path: PathBuf) -> Self {
        Self { launcher, label: label.into(), path, preview: false }
    }
}

/// Environment lookup (injectable for tests).
pub type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

fn subdirs(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    v.sort();
    v
}

const WORLDS: &str = "games/com.mojang/minecraftWorlds";

/// Every worlds folder for this machine's standard locations (existing or
/// not — callers filter by `is_dir`).
pub fn standard_roots(home: Option<&Path>, env: Env<'_>) -> Vec<Root> {
    let mut out = Vec::new();
    if let Some(h) = home {
        java_roots(h, &mut out);
        mcpelauncher_roots(h, env, &mut out);
        bedrock_on_linux_roots(h, &mut out);
        // macOS Java.
        out.push(Root::new(Launcher::JavaVanilla, "Java Launcher", h.join("Library/Application Support/minecraft/saves")));
    }
    windows_roots(env, &mut out);
    out
}

fn java_roots(h: &Path, out: &mut Vec<Root>) {
    out.push(Root::new(Launcher::JavaVanilla, "Java Launcher", h.join(".minecraft/saves")));
    out.push(Root::new(Launcher::JavaVanilla, "Java Launcher (Flatpak)", h.join(".var/app/com.mojang.Minecraft/.minecraft/saves")));
    let instance_bases = [
        (Launcher::Prism, "Prism", h.join(".local/share/PrismLauncher/instances")),
        (Launcher::Prism, "Prism (Flatpak)", h.join(".var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher/instances")),
        (Launcher::MultiMc, "MultiMC", h.join(".local/share/multimc/instances")),
        (Launcher::MultiMc, "MultiMC", h.join(".local/share/MultiMC/instances")),
    ];
    for (l, label, base) in instance_bases {
        for inst in subdirs(&base) {
            let name = inst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let saves = ["minecraft/saves", ".minecraft/saves"].iter().map(|s| inst.join(s)).find(|p| p.is_dir()).unwrap_or_else(|| inst.join("minecraft/saves"));
            out.push(Root::new(l, format!("{label} · {name}"), saves));
        }
    }
    for base in [h.join(".local/share/ModrinthApp/profiles"), h.join(".local/share/com.modrinth.theseus/profiles")] {
        for prof in subdirs(&base) {
            let name = prof.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            out.push(Root::new(Launcher::Modrinth, format!("Modrinth · {name}"), prof.join("saves")));
        }
    }
}

fn mcpelauncher_roots(h: &Path, env: Env<'_>, out: &mut Vec<Root>) {
    let xdg = env("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| h.join(".local/share"));
    let natives = [(xdg.join("mcpelauncher"), "mcpelauncher"), (h.join(".var/app/io.mrarm.mcpelauncher/data/mcpelauncher"), "mcpelauncher (Flatpak)")];
    for (data, label) in natives {
        out.push(Root::new(Launcher::Mcpelauncher, label, data.join(WORLDS)));
        // Profiles with a custom data root.
        if let Ok(ini) = std::fs::read_to_string(data.join("profiles/profiles.ini")) {
            for (profile, dir) in mcpelauncher_custom_dirs(&ini) {
                out.push(Root::new(Launcher::Mcpelauncher, format!("{label} · {profile}"), dir.join(WORLDS)));
            }
        }
    }
}

/// `(profile, data dir)` for every mcpelauncher profile with
/// `dataDirCustom=true` (Qt INI; group names may be %-encoded, values may
/// be quoted).
pub fn mcpelauncher_custom_dirs(ini: &str) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut group = String::new();
    let (mut custom, mut dir): (bool, Option<String>) = (false, None);
    let flush = |group: &str, custom: bool, dir: &Option<String>, out: &mut Vec<(String, PathBuf)>| {
        if custom
            && let Some(d) = dir
            && !d.is_empty()
            && group != "Metadata"
        {
            out.push((group.to_string(), PathBuf::from(d)));
        }
    };
    for line in ini.lines().map(str::trim) {
        if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            flush(&group, custom, &dir, &mut out);
            group = percent_decode(g);
            custom = false;
            dir = None;
        } else if let Some((k, v)) = line.split_once('=') {
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v);
            match k.trim() {
                "dataDirCustom" => custom = v.eq_ignore_ascii_case("true"),
                "dataDir" => dir = Some(v.replace("\\\\", "\\")),
                _ => {}
            }
        }
    }
    flush(&group, custom, &dir, &mut out);
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(hex) = std::str::from_utf8(&b[i + 1..i + 3])
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// GDK layout below a `Minecraft Bedrock[ Preview]` folder: one worlds
/// folder per user id plus `Shared`.
fn gdk_worlds(base: &Path, launcher: Launcher, label: &str, preview: bool, out: &mut Vec<Root>) {
    for user in subdirs(&base.join("Users")) {
        let id = user.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let short: String = if id == "Shared" { "Shared".into() } else { id.chars().take(8).collect() };
        let mut r = Root::new(launcher, format!("{label} · {short}"), user.join(WORLDS));
        r.preview = preview;
        out.push(r);
    }
}

fn bedrock_on_linux_roots(h: &Path, out: &mut Vec<Root>) {
    for base in [h.join(".local/share/bedrock-on-linux"), h.join(".var/app/io.github.wyze3306.BedrockOnLinux/data/bedrock-on-linux")] {
        for pfx in find_prefixes(&base, 3) {
            for user in subdirs(&pfx.join("drive_c/users")) {
                let roaming = user.join("AppData/Roaming");
                gdk_worlds(&roaming.join("Minecraft Bedrock"), Launcher::BedrockOnLinux, "BedrockOnLinux", false, out);
                gdk_worlds(&roaming.join("Minecraft Bedrock Preview"), Launcher::BedrockOnLinux, "BedrockOnLinux Preview", true, out);
            }
        }
    }
}

/// Wine/Proton prefixes (`pfx` folders, or any folder holding `drive_c`)
/// up to `depth` levels below `base`.
fn find_prefixes(base: &Path, depth: u32) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !base.is_dir() {
        return out;
    }
    if base.join("drive_c").is_dir() {
        out.push(base.to_path_buf());
        return out;
    }
    if depth == 0 {
        return out;
    }
    for d in subdirs(base) {
        out.extend(find_prefixes(&d, depth - 1));
    }
    out
}

fn windows_roots(env: Env<'_>, out: &mut Vec<Root>) {
    if let Some(appdata) = env("APPDATA").map(PathBuf::from) {
        out.push(Root::new(Launcher::JavaVanilla, "Java Launcher", appdata.join(".minecraft/saves")));
        gdk_worlds(&appdata.join("Minecraft Bedrock"), Launcher::BedrockWindows, "Bedrock", false, out);
        gdk_worlds(&appdata.join("Minecraft Bedrock Preview"), Launcher::BedrockWindows, "Bedrock Preview", true, out);
        for inst in subdirs(&appdata.join("PrismLauncher/instances")) {
            let name = inst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            out.push(Root::new(Launcher::Prism, format!("Prism · {name}"), inst.join("minecraft/saves")));
        }
        for prof in subdirs(&appdata.join("ModrinthApp/profiles")) {
            let name = prof.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            out.push(Root::new(Launcher::Modrinth, format!("Modrinth · {name}"), prof.join("saves")));
        }
    }
    if let Some(local) = env("LOCALAPPDATA").map(PathBuf::from) {
        let pk = local.join("Packages");
        out.push(Root::new(Launcher::BedrockUwp, "Bedrock (UWP)", pk.join("Microsoft.MinecraftUWP_8wekyb3d8bbwe/LocalState").join(WORLDS)));
        let mut beta = Root::new(Launcher::BedrockUwp, "Bedrock Preview (UWP)", pk.join("Microsoft.MinecraftWindowsBeta_8wekyb3d8bbwe/LocalState").join(WORLDS));
        beta.preview = true;
        out.push(beta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(p: &Path) {
        std::fs::create_dir_all(p).unwrap();
    }

    #[test]
    fn profiles_ini_custom_dirs() {
        let ini = "selected=Default\n[Metadata]\nversion=1\n[Default]\ndataDirCustom=false\ndataDir=/ignored\n[Modded%20Pack]\narch=x86_64\ndataDirCustom=true\ndataDir=\"/mnt/games/mcpe data\"\n[Empty]\ndataDirCustom=true\ndataDir=\n";
        let v = mcpelauncher_custom_dirs(ini);
        assert_eq!(v, vec![("Modded Pack".to_string(), PathBuf::from("/mnt/games/mcpe data"))]);
    }

    #[test]
    fn discovers_linux_layouts() {
        let tmp = std::env::temp_dir().join(format!("omt-sources-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let h = tmp.as_path();
        mk(&h.join(".local/share/PrismLauncher/instances/Fabric 1.21/minecraft/saves"));
        mk(&h.join(".local/share/mcpelauncher/profiles"));
        std::fs::write(h.join(".local/share/mcpelauncher/profiles/profiles.ini"), format!("[Alt]\ndataDirCustom=true\ndataDir={}\n", h.join("alt").display())).unwrap();
        let bol = h.join(".local/share/bedrock-on-linux/compatdata/pfx/drive_c/users/steamuser/AppData/Roaming/Minecraft Bedrock/Users");
        mk(&bol.join("1234567890ABCDEF").join(WORLDS));
        mk(&bol.join("Shared").join(WORLDS));
        let none = |_: &str| None;
        let roots = standard_roots(Some(h), &none);
        let find = |l: Launcher| roots.iter().filter(move |r| r.launcher == l).collect::<Vec<_>>();
        assert!(find(Launcher::Prism).iter().any(|r| r.label == "Prism · Fabric 1.21" && r.path.ends_with("minecraft/saves")));
        assert!(find(Launcher::Mcpelauncher).iter().any(|r| r.label.ends_with("· Alt") && r.path == h.join("alt").join(WORLDS)));
        let b = find(Launcher::BedrockOnLinux);
        assert_eq!(b.len(), 2, "{b:?}");
        assert!(b.iter().any(|r| r.label == "BedrockOnLinux · 12345678"));
        assert!(b.iter().all(|r| r.path.is_dir()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn windows_env_roots_and_tokens() {
        let env = |k: &str| match k {
            "APPDATA" => Some(OsString::from("C:/Users/a/AppData/Roaming")),
            "LOCALAPPDATA" => Some(OsString::from("C:/Users/a/AppData/Local")),
            _ => None,
        };
        let roots = standard_roots(None, &env);
        assert!(roots.iter().any(|r| r.launcher == Launcher::BedrockUwp && r.preview));
        assert!(roots.iter().any(|r| r.launcher == Launcher::JavaVanilla));
        assert_eq!(Launcher::from_token("BOL"), Some(Launcher::BedrockOnLinux));
        assert_eq!(Launcher::from_token("prismlauncher"), Some(Launcher::Prism));
        assert_eq!(Launcher::from_token("mcpe"), Some(Launcher::Mcpelauncher));
    }
}
