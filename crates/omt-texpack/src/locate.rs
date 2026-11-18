//! Finding vanilla resource packs on this computer: installed Bedrock
//! builds (BedrockOnLinux, mcpelauncher, Windows GDK) and a previously
//! downloaded `bedrock-samples` pack in OMT's cache.

use std::path::{Path, PathBuf};

/// A usable pack stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub label: String,
    /// Layers, lowest priority first.
    pub layers: Vec<PathBuf>,
    /// Game version when known (sort key, newest first).
    pub version: Vec<u32>,
}

fn version_of(s: &str) -> Vec<u32> {
    s.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect()
}

/// `vanilla` + `vanilla_<ver>` packs of an installed game's
/// `resource_packs` folder, in override order.
pub fn vanilla_stack(resource_packs: &Path) -> Vec<PathBuf> {
    let mut versioned: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(resource_packs)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let v = name.strip_prefix("vanilla_")?;
            let ver = version_of(v);
            (!ver.is_empty() && e.path().is_dir()).then(|| (ver, e.path()))
        })
        .collect();
    versioned.sort();
    let base = resource_packs.join("vanilla");
    let mut out = Vec::new();
    if base.is_dir() {
        out.push(base);
    }
    out.extend(versioned.into_iter().map(|(_, p)| p));
    out
}

fn subdirs(p: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()
}

/// OMT's download folder for packs.
pub fn download_dir(cache: &Path) -> PathBuf {
    cache.join("texpacks")
}

/// Every pack found, newest game version first (downloads last).
pub fn installed(home: Option<&Path>, cache: Option<&Path>, env: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Vec<Found> {
    let mut out = Vec::new();
    let mut push = |label: String, rp: PathBuf, ver: Vec<u32>| {
        let layers = vanilla_stack(&rp);
        if !layers.is_empty() {
            out.push(Found { label, layers, version: ver });
        }
    };
    if let Some(h) = home {
        // BedrockOnLinux: games/<channel>/<version>/data/resource_packs.
        for base in [h.join(".local/share/bedrock-on-linux/games"), h.join(".var/app/io.github.wyze3306.BedrockOnLinux/data/bedrock-on-linux/games")] {
            for channel in subdirs(&base) {
                let ch = channel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                for ver in subdirs(&channel) {
                    let v = ver.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let label = if ch == "release" { format!("BedrockOnLinux {v}") } else { format!("BedrockOnLinux {ch} {v}") };
                    push(label, ver.join("data/resource_packs"), version_of(&v));
                }
            }
        }
        // mcpelauncher: versions/<version>/assets/assets/resource_packs.
        for data in [h.join(".local/share/mcpelauncher"), h.join(".var/app/io.mrarm.mcpelauncher/data/mcpelauncher")] {
            for ver in subdirs(&data.join("versions")) {
                let v = ver.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let rp = ["assets/assets/resource_packs", "assets/resource_packs"].iter().map(|s| ver.join(s)).find(|p| p.is_dir()).unwrap_or_else(|| ver.join("assets/resource_packs"));
                push(format!("mcpelauncher {v}"), rp, version_of(&v));
            }
        }
    }
    // Windows GDK installs.
    let mut win_roots = vec![PathBuf::from("C:/XboxGames/Minecraft for Windows/Content"), PathBuf::from("C:/XboxGames/Minecraft Preview for Windows/Content")];
    if let Some(pf) = env("ProgramFiles") {
        win_roots.push(PathBuf::from(pf).join("WindowsApps"));
    }
    for r in win_roots {
        if r.join("data/resource_packs").is_dir() {
            push("Minecraft for Windows".into(), r.join("data/resource_packs"), Vec::new());
        }
    }
    out.sort_by(|a, b| b.version.cmp(&a.version));
    if let Some(c) = cache {
        for d in subdirs(&download_dir(c)) {
            let rp = d.join("resource_pack");
            if rp.join("textures").is_dir() {
                let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                out.push(Found { label: format!("Downloaded {name}"), layers: vec![rp], version: version_of(&name) });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_order_and_discovery() {
        let tmp = std::env::temp_dir().join(format!("omt-texpack-locate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let rp = tmp.join(".local/share/bedrock-on-linux/games/release/1.26.63.1/data/resource_packs");
        for p in ["vanilla", "vanilla_1.21.0", "vanilla_1.17.0", "vanilla_1.21.110", "chemistry"] {
            std::fs::create_dir_all(rp.join(p)).unwrap();
        }
        let s = vanilla_stack(&rp);
        let names: Vec<String> = s.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["vanilla", "vanilla_1.17.0", "vanilla_1.21.0", "vanilla_1.21.110"]);
        std::fs::create_dir_all(tmp.join(".local/share/mcpelauncher/versions/1.26.62.1/assets/assets/resource_packs/vanilla")).unwrap();
        let none = |_: &str| None;
        let found = installed(Some(&tmp), None, &none);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].label, "BedrockOnLinux 1.26.63.1", "newest first");
        assert_eq!(found[1].label, "mcpelauncher 1.26.62.1");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
