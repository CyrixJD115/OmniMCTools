//! Real block textures for the UI and the map: the active resource pack
//! (`omt-texpack`), chosen in Settings → Textures. `Auto` uses the newest
//! installed game (BedrockOnLinux / mcpelauncher / Windows) or a pack the
//! user downloaded; without one OMT keeps its procedural art.
//!
//! **Add-on packs** stack on top of that base in the user's order (top
//! wins per texture): packs imported into OMT's add-on folder, packs
//! installed in the game, and — when enabled — the packs the open world
//! carries (topmost, in the world's own order). Marketplace packs are
//! encrypted and never loaded.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use omt_art::{BlockFaces, Image, Material};
use omt_texpack::{Addon, AddonSource, Face, Found, Pack};
use serde::{Deserialize, Serialize};

/// Which pack to use.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TexChoice {
    /// Newest installed / downloaded pack.
    #[default]
    Auto,
    /// A specific pack by label.
    Pack(String),
    /// Procedural art only.
    Off,
}

/// One entry of the user's add-on stack (top first in settings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonPref {
    /// [`Addon::id`].
    pub id: String,
    pub enabled: bool,
}

/// Everything that decides the active textures.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LoadSpec {
    pub choice: TexChoice,
    /// The add-on stack, top first.
    pub stack: Vec<AddonPref>,
    /// The open world whose own packs apply (when enabled).
    pub world: Option<std::path::PathBuf>,
}

static PACK: RwLock<Option<Arc<Pack>>> = RwLock::new(None);
static GEN: AtomicU64 = AtomicU64::new(0);

/// The active pack.
pub fn current() -> Option<Arc<Pack>> {
    PACK.read().ok().and_then(|g| g.clone())
}

/// Bumped whenever the active pack changes (image caches key on it).
pub fn generation() -> u64 {
    GEN.load(Ordering::SeqCst)
}

fn set(p: Option<Arc<Pack>>) {
    if let Ok(mut g) = PACK.write() {
        *g = p;
    }
    GEN.fetch_add(1, Ordering::SeqCst);
}

pub fn cache_root() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("OMT_CACHE_DIR") {
        return std::path::PathBuf::from(d);
    }
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("omt")
}

/// Packs available on this computer (newest first).
pub fn found() -> Vec<Found> {
    let env = |k: &str| std::env::var_os(k);
    omt_texpack::installed(dirs::home_dir().as_deref(), Some(&cache_root()), &env)
}

/// OMT's own add-on folder (imported packs).
pub fn library() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("OMT_DATA_DIR") {
        return std::path::PathBuf::from(d).join("addons");
    }
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("omt").join("addons")
}

/// `com.mojang` folders of the Bedrock games on this computer.
fn game_dirs() -> Vec<(String, std::path::PathBuf)> {
    let env = |k: &str| std::env::var_os(k);
    let mut out: Vec<(String, std::path::PathBuf)> = Vec::new();
    for r in crate::sources::standard_roots(dirs::home_dir().as_deref(), &env) {
        use crate::sources::Launcher;
        if !matches!(r.launcher, Launcher::Mcpelauncher | Launcher::BedrockOnLinux | Launcher::BedrockWindows | Launcher::BedrockUwp) {
            continue;
        }
        // <…>/games/com.mojang/minecraftWorlds → com.mojang; GDK installs
        // also keep shared packs under Users/Shared.
        let Some(cm) = r.path.parent() else { continue };
        let mut dirs = vec![cm.to_path_buf()];
        if let Some(users) = cm.ancestors().nth(3).filter(|u| u.file_name().is_some_and(|n| n == "Users")) {
            dirs.push(users.join("Shared/games/com.mojang"));
        }
        for d in dirs {
            if d.is_dir() && !out.iter().any(|(_, p)| *p == d) {
                out.push((r.launcher.name().to_string(), d));
            }
        }
    }
    out
}

/// Every add-on pack OMT can use: imported, game-installed, and the
/// world's own (applied ones only).
pub fn available(world: Option<&std::path::Path>) -> Vec<Addon> {
    let mut out = omt_texpack::addons::scan_dir(&library(), AddonSource::Imported);
    for (label, d) in game_dirs() {
        for a in omt_texpack::addons::game_packs(&d, &label) {
            if !out.iter().any(|b| b.id == a.id) {
                out.push(a);
            }
        }
    }
    for (label, d) in java_pack_dirs() {
        for a in omt_texpack::addons::java_packs(&d, &label) {
            if !out.iter().any(|b| b.id == a.id) {
                out.push(a);
            }
        }
    }
    if let Some(w) = world {
        out.extend(omt_texpack::addons::world_packs(w));
    }
    out
}

/// `resourcepacks` folders of the Java launchers / instances found (next to
/// each `saves` folder).
fn java_pack_dirs() -> Vec<(String, std::path::PathBuf)> {
    use crate::sources::Launcher;
    let env = |k: &str| std::env::var_os(k);
    let mut out: Vec<(String, std::path::PathBuf)> = Vec::new();
    for r in crate::sources::standard_roots(dirs::home_dir().as_deref(), &env) {
        if !matches!(r.launcher, Launcher::JavaVanilla | Launcher::Prism | Launcher::MultiMc | Launcher::Modrinth) {
            continue;
        }
        let Some(d) = r.path.parent().map(|p| p.join("resourcepacks")) else { continue };
        if d.is_dir() && !out.iter().any(|(_, p)| *p == d) {
            out.push((r.label.clone(), d));
        }
    }
    out
}

/// Load the textures `spec` describes (blocking; call off the UI thread).
/// Returns a status line.
pub fn load(spec: &LoadSpec) -> String {
    let all = found();
    let pick = match &spec.choice {
        TexChoice::Off => {
            set(None);
            return "textures off — using OMT's procedural art".into();
        }
        TexChoice::Auto => all.first().cloned(),
        TexChoice::Pack(label) => all.iter().find(|f| &f.label == label).cloned().or_else(|| all.first().cloned()),
    };
    let t = std::time::Instant::now();
    let addons = available(spec.world.as_deref());
    // Tiers bottom → top: vanilla, the user's stack (stored top first),
    // then the world's packs (stored top first).
    let mut tiers: Vec<Vec<std::path::PathBuf>> = pick.iter().map(|f| f.layers.clone()).collect();
    let mut names: Vec<String> = Vec::new();
    for pref in spec.stack.iter().rev().filter(|p| p.enabled) {
        if let Some(a) = addons.iter().find(|a| a.id == pref.id && a.source != AddonSource::World && !a.encrypted) {
            // Zipped launcher packs: their block textures, extracted once.
            match omt_texpack::addons::materialize(a, &cache_root().join("addon-cache")) {
                Ok(root) if a.archive => tiers.push(vec![root]),
                Ok(_) => tiers.push(a.layers()),
                Err(e) => {
                    names.push(format!("{} (unreadable: {e})", a.name));
                    continue;
                }
            }
            names.push(a.name.clone());
        }
    }
    for a in addons.iter().filter(|a| a.source == AddonSource::World && !a.encrypted).rev() {
        tiers.push(a.layers());
        names.push(format!("{} (world)", a.name));
    }
    if tiers.is_empty() {
        set(None);
        return "no Minecraft resource pack found — install the game, download the vanilla textures or add a pack".into();
    }
    let label = pick.as_ref().map_or_else(|| "Add-ons".to_string(), |f| f.label.clone());
    match Pack::load_tiers(label, &tiers) {
        Some(p) => {
            // One status-bar line: the top two packs by name, then a count.
            let top: Vec<String> = names.iter().rev().take(2).cloned().collect();
            let extra = match names.len() {
                0 => String::new(),
                n if n <= 2 => format!(" + {}", top.join(", ")),
                n => format!(" + {} and {} more add-on(s)", top.join(", "), n - 2),
            };
            let msg = format!("textures: {}{extra} ({} block textures, {} ms)", p.label, p.texture_count(), t.elapsed().as_millis());
            set(Some(Arc::new(p)));
            msg
        }
        None => {
            set(None);
            format!("could not read the pack at {}", tiers[0].first().map(|p| p.display().to_string()).unwrap_or_default())
        }
    }
}

/// Download progress shared with the UI.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

/// Download Mojang's bedrock-samples pack (blocking).
pub fn download(progress: Arc<Progress>) -> Result<String, String> {
    let dir = omt_texpack::locate::download_dir(&cache_root());
    let tag = omt_texpack::download::SAMPLES_TAG;
    omt_texpack::download::fetch_samples(&dir, tag, &mut |done, total| {
        progress.done.store(done, Ordering::Relaxed);
        progress.total.store(total.unwrap_or(0), Ordering::Relaxed);
        !progress.cancel.load(Ordering::Relaxed)
    })
    .map(|p| format!("downloaded the vanilla textures ({tag}) to {}", p.display()))
    .map_err(|e| e.to_string())
}

/// Block + face standing in for an art material.
fn block_of(m: Material) -> (&'static str, Option<[u8; 3]>) {
    use Material::*;
    match m {
        // Tinted only when the pack's texture is gray (Java, Bedrock
        // non-carried); the carried Bedrock variants are already green.
        GrassTop | GrassSide => ("grass_block", Some([0x79, 0xc0, 0x5a])),
        Dirt => ("dirt", None),
        Stone => ("stone", None),
        Deepslate => ("deepslate", None),
        Bedrock => ("bedrock", None),
        Sand => ("sand", None),
        Water => ("water", Some([0x3f, 0x76, 0xe4])),
        LogSide | LogTop => ("oak_log", None),
        Leaves => ("oak_leaves", Some([0x59, 0xae, 0x30])),
        Planks => ("oak_planks", None),
        Netherrack => ("netherrack", None),
        EndStone => ("end_stone", None),
        Obsidian => ("obsidian", None),
        Snow => ("snow", None),
        DiamondOre => ("diamond_ore", None),
        GoldBlock => ("gold_block", None),
        Redstone => ("redstone_block", None),
        Amethyst => ("amethyst_block", None),
        Glass => ("glass", None),
        Copper => ("copper_block", None),
    }
}

fn tinted(img: &Image, tint: Option<[u8; 3]>) -> Image {
    let Some(t) = tint else { return img.clone() };
    let mut out = img.clone();
    for p in out.rgba.as_chunks_mut::<4>().0 {
        for (c, tk) in p.iter_mut().zip(t) {
            *c = ((*c as u32 * tk as u32) / 255) as u8;
        }
    }
    out
}

/// Overlay-style side textures (Bedrock's grass side is a transparent
/// fringe drawn over dirt): composite over `under` when partly clear.
fn over(top: &Image, under: Option<Arc<Image>>) -> Image {
    let Some(under) = under else { return top.clone() };
    if !top.rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 255) || under.w != top.w || under.h != top.h {
        return top.clone();
    }
    let mut out = (*under).clone();
    out.draw(top, 0, 0);
    out
}

/// A texture for an art material from the active pack (landscapes).
pub fn material_texture(m: Material) -> Option<Image> {
    let pack = current()?;
    if m == Material::GrassSide {
        let side = java_grass_side(&pack, pack.face("grass_block", Face::Side, true)?);
        return Some(over(&side, pack.face("dirt", Face::Up, false)));
    }
    let (block, tint) = block_of(m);
    let face = match m {
        Material::GrassSide | Material::LogSide => Face::Side,
        _ => Face::Up,
    };
    // Inventory ("carried") variants are pre-colored for grass/leaves.
    let img = pack.face(block, face, true).or_else(|| pack.face(block, face, false))?;
    let carried_colored = matches!(m, Material::GrassTop | Material::GrassSide | Material::Leaves);
    Some(tinted(&img, if carried_colored && is_colored(&img) { None } else { tint }))
}

/// Java packs draw the grass side as opaque dirt plus a gray fringe
/// (`grass_block_side_overlay`): tint the fringe and lay it on.
fn java_grass_side(pack: &Pack, side: Arc<Image>) -> Image {
    let Some(o) = pack.stem("grass_block_side_overlay") else { return (*side).clone() };
    if o.w != side.w || o.h != side.h {
        return (*side).clone();
    }
    let mut out = (*side).clone();
    out.draw(&tinted(&o, Some([0x79, 0xc0, 0x5a])), 0, 0);
    out
}

/// Grayscale textures need a tint; colored ones don't.
fn is_colored(img: &Image) -> bool {
    img.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).any(|p| p[0].abs_diff(p[1]) > 12 || p[1].abs_diff(p[2]) > 12)
}

/// Isometric faces for an art material from the active pack.
pub fn material_faces(m: Material) -> Option<BlockFaces> {
    let pack = current()?;
    let (block, tint) = block_of(m);
    let top = pack.face(block, Face::Up, true).or_else(|| pack.face(block, Face::Up, false))?;
    let side = pack.face(block, Face::Side, true).or_else(|| pack.face(block, Face::Side, false)).unwrap_or_else(|| top.clone());
    let fix = |img: &Image| if is_colored(img) { img.clone() } else { tinted(img, tint) };
    let (top, mut side) = (fix(&top), fix(&side));
    if block == "grass_block" {
        side = over(&java_grass_side(&pack, Arc::new(side)), pack.face("dirt", Face::Up, false));
    }
    Some(BlockFaces { top, left: side.clone(), right: side })
}
