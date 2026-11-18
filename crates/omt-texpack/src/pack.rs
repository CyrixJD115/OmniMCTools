//! A Bedrock resource-pack *stack* and block → face-texture resolution.
//!
//! Installed games layer `vanilla` + `vanilla_<version>` packs (newer
//! override older); a downloaded `bedrock-samples` resource pack is one
//! already-merged layer. Files are read loose or out of `__brarchive`
//! containers. Resolution order for a block (namespace stripped):
//! `blocks.json` → `terrain_texture.json` key → file; then the texture
//! file index by name (`<n>_top`/`<n>_side`/`<n>`), word-order flips
//! (`oak_log` → `log_oak`), and a few legacy renames (`dark_oak` →
//! `big_oak`, colored wool/concrete/terracotta/glass).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use omt_art::Image;
use serde_json::Value;

use crate::brarchive::Archive;

/// A block face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Face {
    Up,
    Down,
    Side,
}

struct Layer {
    root: PathBuf,
    archives: Mutex<HashMap<PathBuf, Option<Arc<Archive>>>>,
}

impl Layer {
    fn new(root: PathBuf) -> Self {
        Self { root, archives: Mutex::new(HashMap::new()) }
    }

    fn archive(&self, dir: &str) -> Option<Arc<Archive>> {
        let path = self.root.join("__brarchive").join(format!("{dir}.brarchive"));
        let mut g = self.archives.lock().unwrap_or_else(|e| e.into_inner());
        g.entry(path.clone()).or_insert_with(|| Archive::open(&path).ok().map(Arc::new)).clone()
    }

    /// File bytes by pack-relative path (`textures/blocks/stone.png`).
    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        if let Ok(b) = std::fs::read(self.root.join(rel)) {
            return Some(b);
        }
        let (dir, name) = rel.rsplit_once('/')?;
        self.archive(dir)?.get(name).map(<[u8]>::to_vec)
    }

    /// Texture file stems under `textures/blocks` (loose and archived).
    fn block_stems(&self, out: &mut HashMap<String, String>) {
        fn keep(name: &str) -> Option<&str> {
            let stem = name.strip_suffix(".png").or_else(|| name.strip_suffix(".tga"))?;
            (!stem.ends_with("_mers") && !stem.ends_with("_normal") && !stem.ends_with("_heightmap")).then_some(stem)
        }
        let blocks = self.root.join("textures/blocks");
        let mut stack = vec![blocks.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let (Some(n), Ok(rel)) = (p.file_name().and_then(|n| n.to_str()), p.strip_prefix(&self.root))
                    && let Some(stem) = keep(n)
                {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    out.insert(stem.to_string(), rel[..rel.len() - 4].to_string());
                }
            }
        }
        let arch = self.root.join("__brarchive/textures");
        let mut dirs = vec![("textures/blocks".to_string(), arch.join("blocks.brarchive"))];
        for e in std::fs::read_dir(arch.join("blocks")).into_iter().flatten().flatten() {
            let p = e.path();
            if let Some(n) = p.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".brarchive")) {
                dirs.push((format!("textures/blocks/{n}"), p.clone()));
            }
        }
        for (dir, path) in dirs {
            let Ok(a) = Archive::open(&path) else { continue };
            for n in a.names() {
                if let Some(stem) = keep(n) {
                    out.insert(stem.to_string(), format!("{dir}/{stem}"));
                }
            }
        }
    }
}

/// Lenient JSON: `//` and `/* */` comments and trailing commas allowed
/// (the shipped `blocks.json` uses both).
pub fn lenient_json(text: &str) -> Option<Value> {
    let b = text.trim_start_matches('\u{feff}').as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let (mut i, mut in_str) = (0, false);
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c);
            if c == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == b',' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if matches!(b.get(j), Some(b'}') | Some(b']')) {
                i += 1;
                continue;
            }
        }
        if c == b'"' {
            in_str = true;
        }
        out.push(c);
        i += 1;
    }
    serde_json::from_slice(&out).ok()
}

/// Resolved faces by (block, face, carried).
type FaceCache = HashMap<(String, Face, bool), Option<Arc<Image>>>;

/// A resolved pack stack.
pub struct Pack {
    /// Human label ("BedrockOnLinux 1.26.63.1").
    pub label: String,
    layers: Vec<Layer>,
    /// terrain key → texture paths (variants in order).
    terrain: HashMap<String, Vec<String>>,
    /// block → (textures, carried textures) JSON.
    blocks: HashMap<String, (Value, Option<Value>)>,
    /// file stem → pack-relative path without extension.
    stems: HashMap<String, String>,
    cache: Mutex<FaceCache>,
}

impl std::fmt::Debug for Pack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pack").field("label", &self.label).field("layers", &self.layers.len()).field("textures", &self.stems.len()).finish()
    }
}

fn terrain_paths(v: &Value) -> Vec<String> {
    let one = |v: &Value| -> Option<String> {
        match v {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => o.get("path").and_then(Value::as_str).map(str::to_string),
            _ => None,
        }
    };
    match v.get("textures") {
        Some(Value::Array(a)) => a.iter().filter_map(one).collect(),
        Some(other) => one(other).into_iter().collect(),
        None => Vec::new(),
    }
}

impl Pack {
    /// Load a stack (lowest priority first).
    pub fn load(label: impl Into<String>, roots: &[PathBuf]) -> Option<Self> {
        let layers: Vec<Layer> = roots.iter().filter(|r| r.is_dir()).cloned().map(Layer::new).collect();
        if layers.is_empty() {
            return None;
        }
        let mut terrain = HashMap::new();
        let mut blocks = HashMap::new();
        let mut stems = HashMap::new();
        for l in &layers {
            if let Some(t) = l.read("textures/terrain_texture.json").and_then(|b| lenient_json(&String::from_utf8_lossy(&b)))
                && let Some(Value::Object(data)) = t.get("texture_data")
            {
                for (k, v) in data {
                    let p = terrain_paths(v);
                    if !p.is_empty() {
                        terrain.insert(k.clone(), p);
                    }
                }
            }
            if let Some(Value::Object(b)) = l.read("blocks.json").and_then(|b| lenient_json(&String::from_utf8_lossy(&b))) {
                for (k, v) in b {
                    if let Some(t) = v.get("textures") {
                        blocks.insert(k.clone(), (t.clone(), v.get("carried_textures").cloned()));
                    }
                }
            }
            l.block_stems(&mut stems);
        }
        if stems.is_empty() && terrain.is_empty() {
            return None;
        }
        Some(Self { label: label.into(), layers, terrain, blocks, stems, cache: Mutex::new(HashMap::new()) })
    }

    /// Distinct block texture files known.
    pub fn texture_count(&self) -> usize {
        self.stems.len()
    }

    /// Decode a texture by pack path without extension (newest layer
    /// first). Flipbook strips are cropped to their first square frame.
    pub fn image(&self, rel: &str) -> Option<Image> {
        for l in self.layers.iter().rev() {
            for ext in [".png", ".tga"] {
                if let Some(bytes) = l.read(&format!("{rel}{ext}"))
                    && let Ok(mut img) = crate::decode::decode(&bytes)
                {
                    if img.h > img.w && img.w > 0 {
                        img.rgba.truncate((img.w * img.w * 4) as usize);
                        img.h = img.w;
                    }
                    return Some(img);
                }
            }
        }
        None
    }

    fn face_key(textures: &Value, face: Face) -> Option<String> {
        match textures {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => {
                let pick = |k: &str| o.get(k).and_then(Value::as_str).map(str::to_string);
                match face {
                    Face::Up => pick("up"),
                    Face::Down => pick("down"),
                    Face::Side => pick("side").or_else(|| pick("north")).or_else(|| pick("east")),
                }
            }
            _ => None,
        }
    }

    /// Candidate texture paths for a block face, best first.
    pub fn candidates(&self, block: &str, face: Face, carried: bool) -> Vec<String> {
        let n = block.rsplit(':').next().unwrap_or(block);
        let mut out: Vec<String> = Vec::new();
        let from_blocks = |name: &str, variant: usize, out: &mut Vec<String>| {
            if let Some((tex, car)) = self.blocks.get(name) {
                let src = if carried { car.as_ref().unwrap_or(tex) } else { tex };
                if let Some(key) = Self::face_key(src, face)
                    && let Some(paths) = self.terrain.get(&key)
                {
                    out.push(paths.get(variant).unwrap_or(&paths[0]).clone());
                }
            }
        };
        from_blocks(n, 0, &mut out);
        if n == "grass_block" {
            from_blocks("grass", 0, &mut out);
        }
        let mut names = vec![n.to_string()];
        let parts: Vec<&str> = n.split('_').collect();
        if parts.len() >= 2 {
            // oak_log → log_oak, dark_oak_planks → planks_dark_oak.
            let last = parts[parts.len() - 1];
            names.push(format!("{last}_{}", parts[..parts.len() - 1].join("_")));
        }
        for w in names.clone() {
            if w.contains("dark_oak") {
                names.push(w.replace("dark_oak", "big_oak"));
            }
        }
        const COLORS: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
        for c in COLORS {
            if let Some(kind) = n.strip_prefix(&format!("{c}_")) {
                let c2 = if c == "light_gray" { "silver" } else { c };
                match kind {
                    "wool" => names.push(format!("wool_colored_{c2}")),
                    "concrete" => names.push(format!("concrete_{c2}")),
                    "terracotta" => names.push(format!("hardened_clay_stained_{c2}")),
                    "stained_glass" => names.push(format!("glass_{c2}")),
                    "concrete_powder" => names.push(format!("concrete_powder_{c2}")),
                    _ => {}
                }
            }
        }
        // Shapes (stairs, slabs, walls…) look like their base block.
        for suffix in ["_double_slab", "_stairs", "_slab", "_wall", "_fence_gate", "_fence", "_pressure_plate", "_button", "_trapdoor_top"] {
            if let Some(base) = n.strip_suffix(suffix) {
                names.push(base.to_string());
                if let Some(b) = base.strip_suffix("_brick") {
                    names.push(format!("{b}_bricks"));
                }
                names.push(format!("{base}s"));
                names.push(format!("{base}_planks"));
                names.push(format!("planks_{base}"));
            }
        }
        if let Some(sing) = n.strip_suffix('s') {
            names.push(sing.to_string());
        }
        if let Some(c) = n.strip_suffix("_glazed_terracotta") {
            names.push(format!("glazed_terracotta_{}", if c == "light_gray" { "silver" } else { c }));
        }
        match n {
            "dirt_path" => names.push("grass_path".into()),
            "jack_o_lantern" => names.push("pumpkin_face_on".into()),
            "cobweb" => names.push("web".into()),
            "dead_bush" => names.push("deadbush".into()),
            "end_stone_bricks" | "end_stone_brick" => names.push("end_bricks".into()),
            "kelp_plant" | "kelp" => names.push("kelp_a".into()),
            "cave_vines_plant" | "cave_vines" => names.push("cave_vines_body".into()),
            "iron_chain" | "chain" => names.push("chain1".into()),
            "melon_stem" | "attached_melon_stem" | "pumpkin_stem" | "attached_pumpkin_stem" => names.push("melon_stem_disconnected".into()),
            "beetroots" => names.push("beetroots_stage_3".into()),
            "flowering_azalea_leaves" => names.push("azalea_leaves_flowers".into()),
            _ => {}
        }
        match n {
            "grass_block" => names.insert(0, if face == Face::Side { "grass_side_carried".into() } else if carried { "grass_carried".into() } else { "grass_top".into() }),
            "short_grass" | "tall_grass" | "grass" => names.push("tallgrass".into()),
            "water" | "flowing_water" => names.push("water_still".into()),
            "lava" | "flowing_lava" => names.push("lava_still".into()),
            "terracotta" => names.push("hardened_clay".into()),
            "sandstone" => names.push(if face == Face::Side { "sandstone_normal".into() } else { "sandstone_top".into() }),
            _ => {}
        }
        let suffixes: &[&str] = match face {
            Face::Up => &["_top", "", "_side"],
            Face::Down => &["_bottom", "_top", ""],
            Face::Side => &["_side", "", "_top"],
        };
        for w in &names {
            for s in suffixes {
                if let Some(p) = self.stems.get(&format!("{w}{s}")) {
                    out.push(p.clone());
                }
                if carried && let Some(p) = self.stems.get(&format!("{w}{s}_carried")) {
                    out.insert(0, p.clone());
                }
            }
        }
        out.dedup();
        out
    }

    /// The texture of a block face (cached). `carried` prefers the
    /// pre-colored inventory variants (grass, leaves) for UI icons.
    pub fn face(&self, block: &str, face: Face, carried: bool) -> Option<Arc<Image>> {
        let key = (block.to_string(), face, carried);
        if let Some(hit) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return hit.clone();
        }
        let img = self.candidates(block, face, carried).into_iter().find_map(|p| self.image(&p)).map(Arc::new);
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, img.clone());
        img
    }

    /// Roots of this stack.
    pub fn roots(&self) -> Vec<&Path> {
        self.layers.iter().map(|l| l.root.as_path()).collect()
    }
}
