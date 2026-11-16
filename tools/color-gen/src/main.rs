//! `color-gen <client.jar> <out.json>` — derive the map palette OMT's
//! renderer embeds (`crates/omt-render/data/colors.json`).
//!
//! Per block: resolve the default blockstate variant → model chain →
//! the texture on the highest element's `up` face (fallback `particle`),
//! then average its opaque pixels. Per biome: grass / foliage / water
//! colors from `data/minecraft/worldgen/biome/*.json` effects, falling back
//! to the vanilla colormap lookup by temperature × downfall.
//!
//! Output (`blocks[name] = [r, g, b, a, tint]`, tint 0 none / 1 grass /
//! 2 foliage / 3 water; `biomes[name] = [grass, foliage, water]` as 0xRRGGBB).

use std::collections::{BTreeMap, HashMap};
use std::io::Read;

use serde_json::{Map, Value, json};

type Jar = zip::ZipArchive<std::fs::File>;

fn read_entry(jar: &mut Jar, name: &str) -> Option<Vec<u8>> {
    let mut f = jar.by_name(name).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn read_json(jar: &mut Jar, name: &str) -> Option<Value> {
    serde_json::from_slice(&read_entry(jar, name)?).ok()
}

/// `minecraft:block/x` / `block/x` → jar path of that model or texture.
fn asset_path(id: &str, kind: &str, ext: &str) -> String {
    let id = id.strip_prefix("minecraft:").unwrap_or(id);
    format!("assets/minecraft/{kind}/{id}.{ext}")
}

/// RGBA8 pixels of a PNG (any color type), or None.
fn decode_png(bytes: &[u8]) -> Option<Vec<[u8; 4]>> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let data = &buf[..info.buffer_size()];
    let px = match info.color_type {
        png::ColorType::Rgba => data.as_chunks::<4>().0.to_vec(),
        png::ColorType::Rgb => data.as_chunks::<3>().0.iter().map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.as_chunks::<2>().0.iter().map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => data.iter().map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None, // EXPAND removes this
    };
    Some(px)
}

/// Average color of a texture plus a map alpha: cutout textures (pixels
/// either fully clear or fully solid, like leaves) count as opaque when
/// mostly covered; translucent ones keep their mean alpha × coverage.
fn texture_color(jar: &mut Jar, tex: &str) -> Option<[u8; 4]> {
    let px = decode_png(&read_entry(jar, &asset_path(tex, "textures", "png"))?)?;
    let total = px.len().max(1) as f64;
    let solid: Vec<_> = px.iter().filter(|p| p[3] > 0).collect();
    if solid.is_empty() {
        return Some([0, 0, 0, 0]);
    }
    let n = solid.len() as f64;
    let avg = |i: usize| (solid.iter().map(|p| p[i] as f64 * p[i] as f64).sum::<f64>() / n).sqrt() as u8;
    let coverage = n / total;
    let cutout = solid.iter().all(|p| p[3] == 255);
    let mean_a = solid.iter().map(|p| p[3] as f64).sum::<f64>() / n;
    let a = if cutout && coverage > 0.5 { 255.0 } else { mean_a * coverage };
    Some([avg(0), avg(1), avg(2), a.round().clamp(0.0, 255.0) as u8])
}

/// A model resolved through its parent chain.
struct Model {
    textures: HashMap<String, String>,
    elements: Option<Vec<Value>>,
}

fn load_model(jar: &mut Jar, id: &str, depth: u8) -> Option<Model> {
    if depth > 16 {
        return None;
    }
    let v = read_json(jar, &asset_path(id, "models", "json"))?;
    let mut model = match v.get("parent").and_then(Value::as_str) {
        Some(p) => load_model(jar, p, depth + 1).unwrap_or(Model { textures: HashMap::new(), elements: None }),
        None => Model { textures: HashMap::new(), elements: None },
    };
    if let Some(t) = v.get("textures").and_then(Value::as_object) {
        for (k, val) in t {
            // 26.x allows `{ "sprite": "...", "force_translucent": true }`.
            let s = val.as_str().or_else(|| val.get("sprite").and_then(Value::as_str));
            if let Some(s) = s {
                model.textures.insert(k.clone(), s.to_string());
            }
        }
    }
    if let Some(e) = v.get("elements").and_then(Value::as_array) {
        model.elements = Some(e.clone());
    }
    Some(model)
}

fn resolve_texture<'a>(model: &'a Model, mut r: &'a str) -> Option<String> {
    for _ in 0..16 {
        match r.strip_prefix('#') {
            Some(key) => r = model.textures.get(key)?,
            None => return Some(r.to_string()),
        }
    }
    None
}

/// (texture of the top-most `up` face, whether it is a full top face,
/// whether the face is tinted).
fn top_texture(model: &Model) -> Option<(String, bool, bool)> {
    let mut best: Option<(f64, String, bool, bool)> = None;
    for el in model.elements.iter().flatten() {
        let Some(up) = el.pointer("/faces/up") else { continue };
        let top = el.pointer("/to/1").and_then(Value::as_f64).unwrap_or(0.0);
        let Some(tex) = up.get("texture").and_then(Value::as_str).and_then(|t| resolve_texture(model, t)) else {
            continue;
        };
        let from = |i: usize| el.pointer(&format!("/from/{i}")).and_then(Value::as_f64).unwrap_or(0.0);
        let to = |i: usize| el.pointer(&format!("/to/{i}")).and_then(Value::as_f64).unwrap_or(16.0);
        let area = (to(0) - from(0)) * (to(2) - from(2));
        let tinted = up.get("tintindex").is_some();
        if best.as_ref().is_none_or(|b| top > b.0) {
            best = Some((top, tex, area >= 255.0, tinted));
        }
    }
    best.map(|b| (b.1, b.2, b.3))
}

fn default_model(state: &Value) -> Option<String> {
    let pick = |v: &Value| -> Option<String> {
        let v = if let Some(a) = v.as_array() { a.first()? } else { v };
        v.get("model").and_then(Value::as_str).map(str::to_owned)
    };
    if let Some(vars) = state.get("variants").and_then(Value::as_object) {
        if let Some(v) = vars.get("") {
            return pick(v);
        }
        return pick(vars.values().next()?);
    }
    let parts = state.get("multipart")?.as_array()?;
    let part = parts.iter().find(|p| p.get("when").is_none()).or_else(|| parts.first())?;
    pick(part.get("apply")?)
}

/// Biome tint classes the game assigns in code (public block-color
/// knowledge), plus fixed colors for blocks with hard-coded tints.
fn tint_class(name: &str) -> (u8, Option<u32>) {
    const GRASS: &[&str] = &[
        "grass_block", "short_grass", "tall_grass", "fern", "large_fern", "potted_fern", "sugar_cane", "bush",
    ];
    const FOLIAGE: &[&str] =
        &["oak_leaves", "jungle_leaves", "acacia_leaves", "dark_oak_leaves", "mangrove_leaves", "vine"];
    const WATER: &[&str] = &["water", "bubble_column", "water_cauldron"];
    const FIXED: &[(&str, u32)] = &[
        ("spruce_leaves", 0x619961),
        ("birch_leaves", 0x80a755),
        ("lily_pad", 0x208030),
        ("redstone_wire", 0xb00000),
        ("melon_stem", 0x8cb84a),
        ("pumpkin_stem", 0x8cb84a),
        ("attached_melon_stem", 0xe0c71c),
        ("attached_pumpkin_stem", 0xe0c71c),
    ];
    if GRASS.contains(&name) {
        (1, None)
    } else if FOLIAGE.contains(&name) {
        (2, None)
    } else if WATER.contains(&name) {
        (3, None)
    } else if let Some((_, c)) = FIXED.iter().find(|(n, _)| *n == name) {
        (0, Some(*c))
    } else {
        (0, None)
    }
}

fn parse_hex(v: Option<&Value>) -> Option<u32> {
    let v = v?;
    if let Some(n) = v.as_u64() {
        return Some(n as u32);
    }
    u32::from_str_radix(v.as_str()?.trim_start_matches('#'), 16).ok()
}

fn colormap_lookup(map: &[[u8; 4]], temperature: f64, downfall: f64) -> u32 {
    let t = temperature.clamp(0.0, 1.0);
    let d = downfall.clamp(0.0, 1.0) * t;
    let i = ((1.0 - t) * 255.0) as usize;
    let j = ((1.0 - d) * 255.0) as usize;
    let p = map.get((j << 8) | i).copied().unwrap_or([0x91, 0xbd, 0x59, 255]);
    ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: color-gen <client.jar> <out.json>");
        std::process::exit(2);
    }
    let file = std::fs::File::open(&args[1]).expect("open client.jar");
    let mut jar = zip::ZipArchive::new(file).expect("read client.jar");
    let names: Vec<String> = jar.file_names().map(str::to_owned).collect();

    let mut blocks = BTreeMap::new();
    let mut misses = Vec::new();
    for path in names.iter().filter(|n| n.starts_with("assets/minecraft/blockstates/") && n.ends_with(".json")) {
        let block = path.trim_start_matches("assets/minecraft/blockstates/").trim_end_matches(".json").to_string();
        let Some(state) = read_json(&mut jar, path) else { continue };
        let Some(model_id) = default_model(&state) else {
            misses.push(block);
            continue;
        };
        let Some(model) = load_model(&mut jar, &model_id, 0) else {
            misses.push(block);
            continue;
        };
        let (tex, full, face_tinted) = match top_texture(&model) {
            Some(t) => t,
            None => match model.textures.get("particle").and_then(|p| resolve_texture(&model, p)) {
                // No top face (crosses, fluids, entity-rendered blocks):
                // the particle texture stands in, faded unless a fluid.
                Some(p) => (p, matches!(block.as_str(), "water" | "lava" | "bubble_column"), false),
                None => {
                    misses.push(block);
                    continue;
                }
            },
        };
        let Some(mut rgba) = texture_color(&mut jar, &tex) else {
            misses.push(block);
            continue;
        };
        if !full {
            rgba[3] = (rgba[3] as f64 * 0.55) as u8;
        }
        let (mut tint, fixed) = tint_class(&block);
        if tint == 0 && fixed.is_none() && face_tinted {
            tint = 2; // tinted top face without a named class: foliage
        }
        if let Some(c) = fixed {
            // Multiply the (grayscale) texture by the fixed tint.
            for (i, shift) in [16u32, 8, 0].into_iter().enumerate() {
                rgba[i] = ((rgba[i] as u32 * ((c >> shift) & 0xff)) / 255) as u8;
            }
        }
        if block == "water" {
            rgba[3] = 180;
        }
        blocks.insert(format!("minecraft:{block}"), json!([rgba[0], rgba[1], rgba[2], rgba[3], tint]));
    }
    for air in ["air", "cave_air", "void_air", "light", "structure_void", "barrier"] {
        blocks.insert(format!("minecraft:{air}"), json!([0, 0, 0, 0, 0]));
    }

    let grass_map = read_entry(&mut jar, "assets/minecraft/textures/colormap/grass.png").and_then(|b| decode_png(&b)).unwrap_or_default();
    let foliage_map = read_entry(&mut jar, "assets/minecraft/textures/colormap/foliage.png").and_then(|b| decode_png(&b)).unwrap_or_default();
    let mut biomes = BTreeMap::new();
    for path in names.iter().filter(|n| n.starts_with("data/minecraft/worldgen/biome/") && n.ends_with(".json")) {
        let name = path.trim_start_matches("data/minecraft/worldgen/biome/").trim_end_matches(".json");
        let Some(v) = read_json(&mut jar, path) else { continue };
        let t = v.get("temperature").and_then(Value::as_f64).unwrap_or(0.8);
        let d = v.get("downfall").and_then(Value::as_f64).unwrap_or(0.4);
        let fx = v.get("effects").cloned().unwrap_or(Value::Object(Map::new()));
        let mut grass = parse_hex(fx.get("grass_color")).unwrap_or_else(|| colormap_lookup(&grass_map, t, d));
        match fx.get("grass_color_modifier").and_then(Value::as_str) {
            Some("swamp") => grass = 0x6a7039,
            Some("dark_forest") => grass = ((grass & 0xfefefe) + 0x28340a) >> 1,
            _ => {}
        }
        let foliage = parse_hex(fx.get("foliage_color")).unwrap_or_else(|| colormap_lookup(&foliage_map, t, d));
        let water = parse_hex(fx.get("water_color")).unwrap_or(0x3f76e4);
        biomes.insert(format!("minecraft:{name}"), json!([grass, foliage, water]));
    }

    let out = json!({
        "source": "client.jar (pinned) — generated by tools/color-gen",
        "blocks": blocks,
        "biomes": biomes,
    });
    std::fs::write(&args[2], serde_json::to_string(&out).expect("json")).expect("write output");
    eprintln!("{} blocks, {} biomes, {} without a resolvable texture: {:?}", blocks.len(), biomes.len(), misses.len(), misses);
}
