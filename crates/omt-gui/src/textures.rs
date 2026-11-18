//! Real block textures for the UI and the map: the active resource pack
//! (`omt-texpack`), chosen in Settings → Textures. `Auto` uses the newest
//! installed game (BedrockOnLinux / mcpelauncher / Windows) or a pack the
//! user downloaded; without one OMT keeps its procedural art.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use omt_art::{BlockFaces, Image, Material};
use omt_texpack::{Face, Found, Pack};
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

/// Load the chosen pack (blocking; call off the UI thread). Returns a
/// status line.
pub fn load(choice: &TexChoice) -> String {
    let all = found();
    let pick = match choice {
        TexChoice::Off => {
            set(None);
            return "textures off — using OMT's procedural art".into();
        }
        TexChoice::Auto => all.first().cloned(),
        TexChoice::Pack(label) => all.iter().find(|f| &f.label == label).cloned().or_else(|| all.first().cloned()),
    };
    let Some(f) = pick else {
        set(None);
        return "no Minecraft resource pack found — install the game or download the vanilla textures".into();
    };
    let t = std::time::Instant::now();
    match Pack::load(f.label.clone(), &f.layers) {
        Some(p) => {
            let msg = format!("textures: {} ({} block textures, {} ms)", p.label, p.texture_count(), t.elapsed().as_millis());
            set(Some(Arc::new(p)));
            msg
        }
        None => {
            set(None);
            format!("could not read the pack at {}", f.layers.first().map(|p| p.display().to_string()).unwrap_or_default())
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
        GrassTop | GrassSide => ("grass_block", None),
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
        let side = pack.face("grass_block", Face::Side, true)?;
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
        side = over(&side, pack.face("dirt", Face::Up, false));
    }
    Some(BlockFaces { top, left: side.clone(), right: side })
}
