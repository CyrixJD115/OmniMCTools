//! Opt-in download of Mojang's official `bedrock-samples` resource pack
//! (https://github.com/Mojang/bedrock-samples, pinned release) into the
//! user's cache. Those files are "(c) Mojang AB, all rights reserved" and
//! subject to the Minecraft EULA: OMT never ships or redistributes them —
//! this runs only when the user asks, on their machine, and keeps only
//! what the map and icons need (block/item textures + their JSON tables
//! and the license notice).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Pinned latest stable release (2026-10-28).
pub const SAMPLES_TAG: &str = "v1.26.60.5";

pub fn samples_url(tag: &str) -> String {
    format!("https://github.com/Mojang/bedrock-samples/releases/download/{tag}/bedrock-samples-{tag}-full.zip")
}

/// Zip entries worth keeping.
pub fn keep_entry(name: &str) -> bool {
    let n = name.trim_start_matches("./");
    if n == "LICENSE.md" || n.ends_with("/LICENSE.md") && !n.contains("resource_pack/") {
        return true;
    }
    let Some(rest) = n.strip_prefix("resource_pack/") else { return false };
    if matches!(rest, "blocks.json" | "textures/terrain_texture.json" | "textures/item_texture.json" | "textures/flipbook_textures.json" | "manifest.json") {
        return true;
    }
    let tex = rest.starts_with("textures/blocks/") || rest.starts_with("textures/items/");
    let img = rest.ends_with(".png") || rest.ends_with(".tga");
    let pbr = ["_mers.", "_normal.", "_heightmap.", ".texture_set."].iter().any(|s| rest.contains(s));
    tex && img && !pbr
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("network: {0}")]
    Net(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive: {0}")]
    Zip(String),
    #[error("cancelled")]
    Cancelled,
}

/// Download + extract into `<texpacks>/bedrock-samples-<tag>/`.
/// `progress(done, total)` is called as bytes arrive; returning `false`
/// cancels. Returns the pack folder (`…/resource_pack`).
pub fn fetch_samples(texpacks: &Path, tag: &str, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<PathBuf, DownloadError> {
    std::fs::create_dir_all(texpacks)?;
    let dest = texpacks.join(format!("bedrock-samples-{tag}"));
    let part = texpacks.join(format!("bedrock-samples-{tag}.zip.part"));
    let resp = ureq::get(&samples_url(tag)).set("User-Agent", "OmniMCTools").call().map_err(|e| DownloadError::Net(e.to_string()))?;
    let total = resp.header("Content-Length").and_then(|v| v.parse().ok());
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&part)?;
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        if !progress(done, total) {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Cancelled);
        }
    }
    file.sync_all()?;
    drop(file);
    let result = extract(&part, &dest);
    let _ = std::fs::remove_file(&part);
    result?;
    std::fs::write(dest.join("omt-pack.txt"), format!("source = {}\nlicense = see LICENSE.md (Minecraft EULA; not redistributable)\n", samples_url(tag)))?;
    Ok(dest.join("resource_pack"))
}

/// Extract the kept entries of a downloaded zip into `dest`.
pub fn extract(zip_path: &Path, dest: &Path) -> Result<usize, DownloadError> {
    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| DownloadError::Zip(e.to_string()))?;
    let tmp = dest.with_extension("partial");
    let _ = std::fs::remove_dir_all(&tmp);
    let mut n = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| DownloadError::Zip(e.to_string()))?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let name = rel.to_string_lossy().replace('\\', "/");
        if entry.is_dir() || !keep_entry(&name) {
            continue;
        }
        let out = tmp.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(out)?;
        std::io::copy(&mut entry, &mut f)?;
        n += 1;
    }
    let _ = std::fs::remove_dir_all(dest);
    std::fs::rename(&tmp, dest)?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_needed_files() {
        assert!(keep_entry("resource_pack/textures/blocks/stone.png"));
        assert!(keep_entry("resource_pack/textures/blocks/deepslate/deepslate.png"));
        assert!(keep_entry("resource_pack/textures/blocks/leaves_oak.tga"));
        assert!(keep_entry("resource_pack/blocks.json"));
        assert!(keep_entry("LICENSE.md"));
        assert!(!keep_entry("resource_pack/textures/blocks/stone_mers.tga"));
        assert!(!keep_entry("resource_pack/textures/entity/pig.png"));
        assert!(!keep_entry("behavior_pack/entities/pig.json"));
        assert!(!keep_entry("resource_pack/sounds/x.ogg"));
    }

    #[test]
    fn extracts_from_a_zip() {
        let tmp = std::env::temp_dir().join(format!("omt-texpack-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let zp = tmp.join("x.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            for (n, d) in [("resource_pack/textures/blocks/a.png", &b"png"[..]), ("resource_pack/sounds/b.ogg", b"ogg"), ("LICENSE.md", b"(c)")] {
                w.start_file(n, o).unwrap();
                w.write_all(d).unwrap();
            }
            w.finish().unwrap();
        }
        let dest = tmp.join("pack");
        assert_eq!(extract(&zp, &dest).unwrap(), 2);
        assert!(dest.join("resource_pack/textures/blocks/a.png").is_file());
        assert!(!dest.join("resource_pack/sounds").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
