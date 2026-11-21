//! **Add-on resource packs** layered over the vanilla textures: Bedrock
//! packs (`manifest.json` with a `resources` module; `.mcpack`,
//! `.mcaddon`, `.zip` or a folder) and Java resource packs
//! (`pack.mcmeta` + `assets/minecraft/textures`). Sources: packs the user
//! imports into OMT's data folder, packs installed in the game
//! (`com.mojang/resource_packs`, `development_resource_packs`), and the
//! packs a Bedrock world carries (`world_resource_packs.json` →
//! `<world>/resource_packs`, subpacks included).
//!
//! Marketplace packs are encrypted (`contents.json` is not JSON); they are
//! listed as such and never loaded.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::pack::lenient_json;

/// Pack format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddonKind {
    Bedrock,
    Java,
}

/// Where a pack was found.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AddonSource {
    /// Imported into OMT's own add-on folder (removable from OMT).
    Imported,
    /// Installed in a game (label of the game data folder).
    Game(String),
    /// Carried by a world.
    World,
}

/// One add-on resource pack.
#[derive(Debug, Clone, PartialEq)]
pub struct Addon {
    /// Stable id: the Bedrock header uuid, or `java:<folder name>`.
    pub id: String,
    pub name: String,
    pub description: String,
    /// `1.2.6` (Bedrock header version; empty for Java).
    pub version: String,
    pub kind: AddonKind,
    /// The pack's root folder.
    pub root: PathBuf,
    pub source: AddonSource,
    /// `pack_icon.png` / `pack.png` when present.
    pub icon: Option<PathBuf>,
    /// Marketplace (encrypted): listed, never loaded.
    pub encrypted: bool,
    /// Active subpack folder (world packs pick one), layered on top.
    pub subpack: Option<String>,
    /// `root` is a `.zip` (launcher resource-pack folders): extracted into
    /// OMT's cache on first use ([`materialize`]).
    pub archive: bool,
    /// The pack ships block textures (computed when read).
    pub block_textures: bool,
}

impl Addon {
    /// Folders to load, lowest priority first (the pack, then its subpack).
    pub fn layers(&self) -> Vec<PathBuf> {
        let mut v = vec![self.root.clone()];
        if let Some(s) = &self.subpack {
            let p = self.root.join("subpacks").join(s);
            if p.is_dir() {
                v.push(p);
            }
        }
        v
    }

    /// Whether the pack can change block textures at all.
    pub fn has_block_textures(&self) -> bool {
        self.block_textures
    }
}

/// Folders / zip prefixes holding block textures, Bedrock and Java.
const BLOCK_DIRS: [&str; 4] = ["textures/blocks", "textures/terrain_texture.json", "assets/minecraft/textures/block", "assets/minecraft/textures/blocks"];

fn folder_has_blocks(root: &Path) -> bool {
    BLOCK_DIRS.iter().any(|d| root.join(d).exists())
}

fn version_str(v: Option<&Value>) -> String {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_u64).map(|n| n.to_string()).collect::<Vec<_>>().join("."),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Bedrock `pack.name` style keys resolve through `texts/en_US.lang`.
fn localized(root: &Path, s: &str) -> String {
    if !s.starts_with("pack.") {
        return s.to_string();
    }
    let lang = std::fs::read_to_string(root.join("texts/en_US.lang")).unwrap_or_default();
    lang.lines().find_map(|l| l.strip_prefix(s).and_then(|r| r.strip_prefix('=')).map(|v| v.split('\t').next().unwrap_or(v).trim().to_string())).unwrap_or_else(|| s.to_string())
}

/// Java text components → plain text.
fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(plain).collect(),
        Value::Object(o) => {
            let mut s = o.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
            if let Some(Value::Array(extra)) = o.get("extra") {
                s.extend(extra.iter().map(plain));
            }
            s
        }
        _ => String::new(),
    }
}

/// Strip `§x` formatting codes.
fn unformat(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut skip = false;
    for c in s.chars() {
        if skip {
            skip = false;
        } else if c == '§' {
            skip = true;
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Marketplace packs ship an encrypted `contents.json`.
pub fn is_encrypted(root: &Path) -> bool {
    match std::fs::read(root.join("contents.json")) {
        Ok(b) => {
            let t = b.iter().position(|c| !c.is_ascii_whitespace() && *c != 0xEF && *c != 0xBB && *c != 0xBF);
            !matches!(t.map(|i| b[i]), Some(b'{') | Some(b'['))
        }
        Err(_) => false,
    }
}

/// Read the pack rooted at `root` (`None` = not a resource pack).
pub fn read_pack(root: &Path, source: AddonSource) -> Option<Addon> {
    let folder = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(m) = std::fs::read(root.join("manifest.json")).ok().and_then(|b| lenient_json(&String::from_utf8_lossy(&b))) {
        let header = m.get("header")?;
        let modules = m.get("modules").and_then(Value::as_array).cloned().unwrap_or_default();
        // Behaviour-only packs carry no textures.
        if !modules.iter().any(|md| md.get("type").and_then(Value::as_str) == Some("resources")) {
            return None;
        }
        let name = header.get("name").and_then(Value::as_str).map_or_else(|| folder.clone(), |n| unformat(&localized(root, n)));
        let description = header.get("description").and_then(Value::as_str).map(|d| unformat(&localized(root, d))).unwrap_or_default();
        return Some(Addon {
            id: header.get("uuid").and_then(Value::as_str).unwrap_or(&folder).to_ascii_lowercase(),
            name,
            description,
            version: version_str(header.get("version")),
            kind: AddonKind::Bedrock,
            root: root.to_path_buf(),
            source,
            icon: Some(root.join("pack_icon.png")).filter(|p| p.is_file()),
            encrypted: is_encrypted(root),
            subpack: None,
            archive: false,
            block_textures: folder_has_blocks(root),
        });
    }
    let meta = std::fs::read(root.join("pack.mcmeta")).ok().and_then(|b| lenient_json(&String::from_utf8_lossy(&b)))?;
    let description = meta.get("pack").and_then(|p| p.get("description")).map(plain).map(|d| unformat(&d)).unwrap_or_default();
    Some(Addon {
        id: format!("java:{folder}"),
        name: folder,
        description,
        version: String::new(),
        kind: AddonKind::Java,
        root: root.to_path_buf(),
        source,
        icon: Some(root.join("pack.png")).filter(|p| p.is_file()),
        encrypted: false,
        subpack: None,
        archive: false,
        block_textures: folder_has_blocks(root),
    })
}

/// Every resource pack directly inside `dir` (sorted by name).
pub fn scan_dir(dir: &Path, source: AddonSource) -> Vec<Addon> {
    let mut out: Vec<Addon> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).filter_map(|p| read_pack(&p, source.clone())).collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// Java resource packs in a launcher's `resourcepacks` folder: unpacked
/// folders and `.zip` files (read from the zip index + `pack.mcmeta`
/// only; nothing is extracted until the pack is used).
pub fn java_packs(dir: &Path, label: &str) -> Vec<Addon> {
    let source = AddonSource::Game(label.to_string());
    let mut out: Vec<Addon> = scan_dir(dir, source.clone()).into_iter().filter(|a| a.kind == AddonKind::Java).collect();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) {
            continue;
        }
        let Ok(f) = std::fs::File::open(&p) else { continue };
        let Ok(mut z) = zip::ZipArchive::new(std::io::BufReader::new(f)) else { continue };
        let block_textures = z.file_names().any(|n| n.starts_with("assets/minecraft/textures/block"));
        let mut meta = String::new();
        let Some(m) = z.by_name("pack.mcmeta").ok().and_then(|mut f| f.read_to_string(&mut meta).ok().and_then(|_| lenient_json(&meta))) else { continue };
        let name = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        out.push(Addon {
            id: format!("java:{name}"),
            description: m.get("pack").and_then(|p| p.get("description")).map(plain).map(|d| unformat(&d)).unwrap_or_default(),
            name,
            version: String::new(),
            kind: AddonKind::Java,
            root: p.clone(),
            source: source.clone(),
            icon: None,
            encrypted: false,
            subpack: None,
            archive: true,
            block_textures,
        });
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// The folder to load an add-on from: itself, or for a zipped pack its
/// block textures extracted under `cache` (reused while the zip is
/// unchanged).
pub fn materialize(a: &Addon, cache: &Path) -> Result<PathBuf, ImportError> {
    if !a.archive {
        return Ok(a.root.clone());
    }
    let meta = std::fs::metadata(&a.root)?;
    let stamp = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let mut h: u64 = 0xcbf29ce484222325;
    for b in a.root.to_string_lossy().bytes().chain(meta.len().to_le_bytes()).chain(stamp.to_le_bytes()) {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let dest = cache.join(format!("{}-{h:016x}", safe_name(&a.name)));
    if dest.join(".omt-complete").is_file() {
        return Ok(dest);
    }
    let _ = std::fs::remove_dir_all(&dest);
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(&a.root)?)).map_err(|e| ImportError::Zip(e.to_string()))?;
    for i in 0..z.len() {
        let mut entry = z.by_index(i).map_err(|e| ImportError::Zip(e.to_string()))?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let r = rel.to_string_lossy().replace('\\', "/");
        if entry.is_dir() || !(r == "pack.mcmeta" || r == "pack.png" || r.starts_with("assets/minecraft/textures/block")) {
            continue;
        }
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data)?;
        std::fs::write(out, data)?;
    }
    std::fs::write(dest.join(".omt-complete"), b"")?;
    Ok(dest)
}

/// Packs installed in a game's `com.mojang` folder.
pub fn game_packs(com_mojang: &Path, label: &str) -> Vec<Addon> {
    let mut out = scan_dir(&com_mojang.join("resource_packs"), AddonSource::Game(label.to_string()));
    out.extend(scan_dir(&com_mojang.join("development_resource_packs"), AddonSource::Game(format!("{label} (development)"))));
    out
}

/// The packs a Bedrock world applies, top priority first (the order of
/// `world_resource_packs.json`), with their chosen subpacks.
pub fn world_packs(world: &Path) -> Vec<Addon> {
    let Some(Value::Array(list)) = std::fs::read(world.join("world_resource_packs.json")).ok().and_then(|b| lenient_json(&String::from_utf8_lossy(&b))) else {
        return Vec::new();
    };
    let carried = scan_dir(&world.join("resource_packs"), AddonSource::World);
    list.iter()
        .filter_map(|e| {
            let id = e.get("pack_id").and_then(Value::as_str)?.to_ascii_lowercase();
            let mut a = carried.iter().find(|a| a.id == id)?.clone();
            a.subpack = e.get("subpack").and_then(Value::as_str).map(str::to_string);
            Some(a)
        })
        .collect()
}

/// Import errors.
#[derive(Debug)]
pub enum ImportError {
    Io(std::io::Error),
    Zip(String),
    /// Nothing in the file is a resource pack.
    NoPack,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Zip(e) => write!(f, "not a readable archive: {e}"),
            Self::NoPack => write!(f, "no resource pack inside (behaviour-only add-on?)"),
        }
    }
}

impl From<std::io::Error> for ImportError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let p = e.path();
        if e.file_type()?.is_dir() {
            copy_dir(&p, &to.join(e.file_name()))?;
        } else {
            std::fs::copy(&p, to.join(e.file_name()))?;
        }
    }
    Ok(())
}

/// Unpack a zip (nested `.mcpack`s of an `.mcaddon` included) into `dest`.
fn unzip(bytes: &[u8], dest: &Path, depth: u8) -> Result<(), ImportError> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| ImportError::Zip(e.to_string()))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| ImportError::Zip(e.to_string()))?;
        // `enclosed_name` rejects absolute paths and `..` (zip slip).
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data)?;
        let lower = rel.to_string_lossy().to_ascii_lowercase();
        if depth < 2 && (lower.ends_with(".mcpack") || lower.ends_with(".zip")) {
            let stem = rel.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| format!("pack{i}"));
            unzip(&data, &dest.join(stem), depth + 1)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out, data)?;
    }
    Ok(())
}

/// Every pack root under `dir` (the folder itself, or nested folders as
/// `.mcaddon`s and zipped Java packs lay them out), at most 3 levels down.
fn pack_roots(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    if dir.join("manifest.json").is_file() || dir.join("pack.mcmeta").is_file() {
        out.push(dir.to_path_buf());
        return;
    }
    if depth == 0 {
        return;
    }
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    subs.sort();
    for s in subs {
        pack_roots(&s, depth - 1, out);
    }
}

fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') { c } else { '_' }).collect();
    let n = n.trim().trim_matches('.').to_string();
    if n.is_empty() { "pack".into() } else { n }
}

/// Import a `.mcpack` / `.mcaddon` / `.zip` file or a pack folder into
/// `library` (OMT's add-on folder). Returns the resource packs it held
/// (behaviour packs are skipped; an existing copy of a pack is replaced).
pub fn import(src: &Path, library: &Path) -> Result<Vec<Addon>, ImportError> {
    std::fs::create_dir_all(library)?;
    let stage = library.join(format!(".import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    let staged = (|| {
        if src.is_dir() {
            copy_dir(src, &stage.join(src.file_name().unwrap_or_default()))
                .map_err(ImportError::from)
        } else {
            unzip(&std::fs::read(src)?, &stage.join(src.file_stem().unwrap_or_default()), 0)
        }
    })();
    if let Err(e) = staged {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    let mut roots = Vec::new();
    pack_roots(&stage, 4, &mut roots);
    let mut out = Vec::new();
    for r in roots {
        let Some(a) = read_pack(&r, AddonSource::Imported) else { continue };
        let dest = library.join(safe_name(&if a.kind == AddonKind::Bedrock { a.id.clone() } else { a.name.clone() }));
        let _ = std::fs::remove_dir_all(&dest);
        if std::fs::rename(&r, &dest).is_err() {
            copy_dir(&r, &dest)?;
        }
        if let Some(a) = read_pack(&dest, AddonSource::Imported) {
            out.push(a);
        }
    }
    let _ = std::fs::remove_dir_all(&stage);
    if out.is_empty() {
        return Err(ImportError::NoPack);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "addons_tests.rs"]
mod tests;
